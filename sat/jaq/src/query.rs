use core::{Ctx, DataT, Lut, Vars, data::HasLut, native};
use jaq_all::{jaq_core as core, jaq_std as stdlib, json};
use json::{Num, Val};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::File,
    io::{self, Read, Write},
};
use stdlib::input::{HasInputs, Inputs, RcIter};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_VALUES: usize = 100_000;

#[derive(Default)]
pub struct Request {
    pub filter: Option<String>,
    pub input: Option<String>,
    pub files: Vec<String>,
    pub from_file: Option<String>,
    pub arg: Vec<String>,
    pub argjson: Vec<String>,
    pub slurpfile: Vec<String>,
    pub rawfile: Vec<String>,
    pub args: Option<Vec<String>>,
    pub jsonargs: Option<Vec<String>>,
    pub null_input: bool,
    pub raw_input: bool,
    pub slurp: bool,
    pub raw_output: bool,
    pub join_output: bool,
    pub raw_output0: bool,
    pub compact_output: bool,
    pub sort_keys: bool,
    pub ascii_output: bool,
    pub tab: bool,
    pub indent: u32,
    pub exit_status: bool,
}

struct Kind;
struct Data<'a> {
    lut: &'a Lut<Kind>,
    inputs: Inputs<'a, Val>,
    filename: &'a RefCell<Val>,
}
impl DataT for Kind {
    type V<'a> = Val;
    type Data<'a> = &'a Data<'a>;
}
impl<'a> HasLut<'a, Kind> for &'a Data<'a> {
    fn lut(&self) -> &'a Lut<Kind> {
        self.lut
    }
}
impl<'a> HasInputs<'a, Val> for &'a Data<'a> {
    fn inputs(&self) -> Inputs<'a, Val> {
        self.inputs
    }
}

fn functions() -> impl Iterator<Item = native::Fun<Kind>> {
    let input = stdlib::input::funs::<Kind>()
        .into_vec()
        .into_iter()
        .map(native::run::<Kind>);
    let filename = native::run::<Kind>(("input_filename", native::v(0), |cv| {
        Box::new(std::iter::once(Ok(cv.0.data().filename.borrow().clone())))
    }));
    core::funs()
        .chain(stdlib::funs())
        .chain(json::funs())
        .chain(input)
        .chain([filename])
}

fn read_file(path: &str, budget: &mut usize) -> Result<String, String> {
    let file = File::open(path).map_err(|_| "cannot read granted file (missing or denied)")?;
    if !file
        .metadata()
        .map_err(|_| "cannot inspect input file")?
        .is_file()
    {
        return Err("input must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take((*budget + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read input file")?;
    if bytes.len() > *budget {
        return Err("aggregate input exceeds 8 MiB".into());
    }
    *budget -= bytes.len();
    String::from_utf8(bytes).map_err(|_| "input files must be UTF-8".into())
}

fn values(text: &str) -> Result<Vec<Val>, String> {
    // Validate JSON first: jaq's own reader also accepts XJON extensions. Keep
    // this interface JSON-only, while preserving jaq's number representation.
    let mut reader = serde_json::Deserializer::from_str(text).into_iter::<serde_json::Value>();
    let mut count = 0;
    for value in &mut reader {
        value.map_err(|e| format!("invalid JSON: {e}"))?;
        count += 1;
        if count > MAX_VALUES {
            return Err("input exceeds 100000 values".into());
        }
    }
    json::read::parse_many(text.as_bytes())
        .map(|r| r.map_err(|e| format!("invalid JSON: {e}")))
        .collect()
}

fn one_value(text: &str) -> Result<Val, String> {
    let mut result = values(text)?;
    if result.len() != 1 {
        return Err("expected exactly one JSON value".into());
    }
    Ok(result.remove(0))
}

fn binding(text: &str) -> Result<(&str, &str), String> {
    let (name, value) = text
        .split_once('=')
        .ok_or("variable bindings use NAME=VALUE")?;
    if name.is_empty()
        || !name
            .chars()
            .enumerate()
            .all(|(i, ch)| ch == '_' || ch.is_ascii_alphabetic() || (i > 0 && ch.is_ascii_digit()))
    {
        return Err("invalid variable name".into());
    }
    Ok((name, value))
}

struct Buffer(Vec<u8>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_BYTES {
            return Err(io::Error::other("output exceeds 8 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// jq prints NaN as null and clamps infinities. Reject jaq-only value types
// instead of emitting XJON while advertising JSON output.
fn json_value(value: Val) -> Result<Val, String> {
    Ok(match value {
        Val::Num(Num::Float(f)) if f.is_nan() => Val::Null,
        Val::Num(Num::Float(f)) if f.is_infinite() => Val::Num(Num::Float(f.signum() * f64::MAX)),
        Val::BStr(_) => {
            return Err("byte strings are a jaq extension; convert to JSON text first".into());
        }
        Val::TStr(bytes) => Val::from(String::from_utf8_lossy(&bytes).into_owned()),
        Val::Arr(array) => array
            .iter()
            .cloned()
            .map(json_value)
            .collect::<Result<Val, _>>()?,
        Val::Obj(object) => {
            let pairs = object
                .iter()
                .map(|(key, value)| {
                    if !matches!(key, Val::TStr(_)) {
                        return Err("JSON object keys must be strings".into());
                    }
                    Ok((json_value(key.clone())?, json_value(value.clone())?))
                })
                .collect::<Result<_, String>>()?;
            Val::obj(pairs)
        }
        value => value,
    })
}

fn ascii(text: &str) -> String {
    use std::fmt::Write as _;
    let mut output = String::new();
    for ch in text.chars() {
        if ch.is_ascii() {
            output.push(ch);
        } else {
            for unit in ch.encode_utf16(&mut [0; 2]) {
                write!(output, "\\u{unit:04x}").unwrap();
            }
        }
    }
    output
}

pub fn run(request: Request) -> Result<String, String> {
    if request.indent > 8 {
        return Err("indent must be 0..8".into());
    }
    if request.input.is_some() && !request.files.is_empty() {
        return Err("choose --input/--stdin or --files".into());
    }
    if request.filter.is_some() && request.from_file.is_some() {
        return Err("choose a filter or --from-file".into());
    }
    if request.args.is_some() && request.jsonargs.is_some() {
        return Err("choose --args or --jsonargs".into());
    }
    if request.join_output && request.raw_output0 {
        return Err("--join-output conflicts with --raw-output0".into());
    }
    if request.files.len() > 128 {
        return Err("at most 128 input files".into());
    }
    let mut budget = MAX_BYTES;
    let filter = if let Some(path) = &request.from_file {
        read_file(path, &mut budget)?
    } else {
        request.filter.clone().unwrap_or_else(|| ".".into())
    };
    if filter.len() > 65536 {
        return Err("filter exceeds 64 KiB".into());
    }
    let mut variables = BTreeMap::new();
    for text in &request.arg {
        let (name, value) = binding(text)?;
        variables.insert(name.to_owned(), Val::from(value.to_owned()));
    }
    for text in &request.argjson {
        let (name, value) = binding(text)?;
        variables.insert(name.to_owned(), one_value(value)?);
    }
    for (bindings, raw) in [(&request.slurpfile, false), (&request.rawfile, true)] {
        for text in bindings {
            let (name, path) = binding(text)?;
            let text = read_file(path, &mut budget)?;
            variables.insert(
                name.into(),
                if raw {
                    Val::from(text)
                } else {
                    values(&text)?.into_iter().collect()
                },
            );
        }
    }
    let named: json::Map<_, _> = variables
        .iter()
        .map(|(name, value)| (Val::from(name.clone()), value.clone()))
        .collect();
    let positional: Val = if let Some(args) = request.args {
        args.into_iter().map(Val::from).collect()
    } else {
        request
            .jsonargs
            .unwrap_or_default()
            .iter()
            .map(|text| one_value(text))
            .collect::<Result<Val, _>>()?
    };
    variables.entry("ARGS".into()).or_insert_with(|| {
        Val::obj(
            [
                (Val::from("named".to_owned()), Val::obj(named)),
                (Val::from("positional".to_owned()), positional),
            ]
            .into_iter()
            .collect(),
        )
    });
    variables.entry("ENV".into()).or_insert_with(|| {
        Val::obj(
            std::env::vars()
                .map(|(k, v)| (Val::from(k), Val::from(v)))
                .collect(),
        )
    });
    let names: Vec<_> = variables.keys().cloned().collect();
    let filter =
        jaq_all::compile_with(&filter, jaq_all::defs(), functions(), &names).map_err(|_| {
            "invalid filter, undefined function/variable, or unsupported module import".to_owned()
        })?;

    let mut sources = Vec::new();
    if let Some(input) = request.input {
        if input.len() > budget {
            return Err("aggregate input exceeds 8 MiB".into());
        }
        sources.push(("<stdin>".to_owned(), input));
    } else if !request.files.is_empty() {
        for path in &request.files {
            sources.push((path.clone(), read_file(path, &mut budget)?));
        }
    } else if !request.null_input {
        return Err("provide --input, --stdin, or --files (or use --null-input true)".into());
    }
    let mut inputs = Vec::new();
    if request.raw_input && request.slurp {
        let text = sources
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<String>();
        inputs.push((
            sources
                .last()
                .map_or("<stdin>", |(name, _)| name.as_str())
                .to_owned(),
            Val::from(text),
        ));
    } else {
        for (filename, text) in sources {
            let source_values: Vec<Val> = if request.raw_input {
                text.split_terminator('\n')
                    .map(|line| Val::from(line.to_owned()))
                    .collect()
            } else {
                values(&text)?
            };
            if inputs.len().saturating_add(source_values.len()) > MAX_VALUES {
                return Err("input exceeds 100000 values".into());
            }
            inputs.extend(
                source_values
                    .into_iter()
                    .map(|value| (filename.clone(), value)),
            );
        }
        if request.slurp {
            let filename = inputs
                .last()
                .map_or("<stdin>", |(name, _)| name.as_str())
                .to_owned();
            inputs = vec![(
                filename,
                inputs.into_iter().map(|(_, value)| value).collect(),
            )];
        }
    }
    let current_filename = RefCell::new(Val::Null);
    let iterator = inputs.into_iter().map(|(filename, value)| {
        *current_filename.borrow_mut() = Val::from(filename);
        Ok(value)
    });
    let boxed: Box<dyn Iterator<Item = Result<Val, String>>> = Box::new(iterator);
    let inputs = RcIter::new(boxed);
    let data = Data {
        lut: &filter.lut,
        inputs: &inputs,
        filename: &current_filename,
    };
    let ctx = Ctx::<Kind>::new(&data, Vars::new(variables.into_values()));
    let mut output = Buffer(Vec::new());
    let pp = json::write::Pp {
        indent: (!request.compact_output).then(|| {
            if request.tab {
                "\t".into()
            } else {
                " ".repeat(request.indent as usize)
            }
        }),
        sort_keys: request.sort_keys,
        sep_space: !request.compact_output,
        ..Default::default()
    };
    let mut last = None;
    let mut produced = 0;
    let null = RcIter::new(std::iter::once(Ok(Val::Null)));
    let main_inputs: &RcIter<dyn Iterator<Item = Result<Val, String>>> =
        if request.null_input { &null } else { &inputs };
    'run: for value in main_inputs {
        for value in filter.id.run((ctx.clone(), value?)) {
            let value = match value {
                Ok(value) => value,
                Err(exception) => match exception.get_halt() {
                    Ok(0) => break 'run,
                    Ok(code) => return Err(format!("filter halted with status {code}")),
                    Err(exception) => {
                        return Err(exception
                            .get_err()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|_| "filter execution failed".into()));
                    }
                },
            };
            last = Some(!matches!(value, Val::Null | Val::Bool(false)));
            produced += 1;
            if produced > MAX_VALUES {
                return Err("output exceeds 100000 values".into());
            }
            let raw = request.raw_output || request.join_output || request.raw_output0;
            if raw && let Val::TStr(bytes) = &value {
                if request.raw_output0 && bytes.contains(&0) {
                    return Err("raw-output0 rejects strings containing NUL".into());
                }
                output.write_all(bytes).map_err(|e| e.to_string())?;
            } else {
                let value = json_value(value)?;
                if request.ascii_output {
                    let mut rendered = Buffer(Vec::new());
                    json::write::write(&mut rendered, &pp, 0, &value).map_err(|e| e.to_string())?;
                    let text = String::from_utf8(rendered.0).map_err(|_| "output is not UTF-8")?;
                    output
                        .write_all(ascii(&text).as_bytes())
                        .map_err(|e| e.to_string())?;
                } else {
                    json::write::write(&mut output, &pp, 0, &value).map_err(|e| e.to_string())?;
                }
            }
            if request.raw_output0 {
                output.write_all(b"\0").map_err(|e| e.to_string())?;
            } else if !request.join_output {
                output.write_all(b"\n").map_err(|e| e.to_string())?;
            }
        }
    }
    if request.exit_status && last != Some(true) {
        return Err(if last.is_none() {
            "no output"
        } else {
            "last output is false or null"
        }
        .into());
    }
    String::from_utf8(output.0).map_err(|_| "WIT text output requires valid UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn query(filter: &str, input: &str) -> Request {
        Request {
            filter: Some(filter.into()),
            input: Some(input.into()),
            indent: 2,
            ..Default::default()
        }
    }
    #[test]
    fn filters_and_shared_inputs() {
        assert_eq!(run(query("map(. * 2) | add", "[1,2,3]")).unwrap(), "12\n");
        assert_eq!(
            run(query("[., inputs]", "1 2 3")).unwrap(),
            "[\n  1,\n  2,\n  3\n]\n"
        );
    }
    #[test]
    fn join_and_empty_have_no_extra_newline() {
        let mut request = query(".[]", r#"["a", "b"]"#);
        request.join_output = true;
        assert_eq!(run(request).unwrap(), "ab");
        assert_eq!(run(query("empty", "null")).unwrap(), "");
        assert_eq!(run(query("halt", "null")).unwrap(), "");
    }
    #[test]
    fn json_output_rejects_extensions_and_maps_nonfinite_numbers() {
        assert_eq!(run(query("nan", "null")).unwrap(), "null\n");
        assert!(
            run(query("{(0):1}", "null"))
                .unwrap_err()
                .contains("keys must be strings")
        );
        assert!(run(query(".", "# not JSON\n1")).is_err());
    }
}
