use clap::Parser;
use std::{
    ffi::{CStr, CString, c_char},
    fs::{File, OpenOptions},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
};

wit_bindgen::generate!({path:"wit",world:"tesseract-cli"});
struct ComponentImpl;
impl Guest for ComponentImpl {
    fn run(args: Vec<String>) -> i32 {
        run_cli(args)
    }
}
export!(ComponentImpl);

const MODEL: &[u8] = include_bytes!("../target/eng.traineddata");
const INPUT_LIMIT: usize = 32 * 1024 * 1024;
const PIXEL_LIMIT: u64 = 20_000_000;

unsafe extern "C" {
    fn sat_ocr(
        gray: *const u8,
        width: u32,
        height: u32,
        model: *const u8,
        model_size: i32,
        language: *const c_char,
        psm: i32,
        format: i32,
        output: *mut *mut c_char,
    ) -> i32;
    fn sat_ocr_free(output: *mut c_char);
}

#[derive(Parser)]
#[command(
    name = "tesseract",
    version = "5.5.4 (SAT image OCR profile 0.1.0)",
    about = "Tesseract OCR in WASI. Embedded English model; PNG/JPEG/PNM/BMP input; txt/tsv/hocr output."
)]
struct Cli {
    /// Input image path, or stdin/- for image bytes.
    input: String,
    /// Output basename (adds .txt/.tsv/.hocr), or stdout/-.
    #[arg(default_value = "stdout")]
    outputbase: String,
    /// Language: embedded eng, or the language of --model.
    #[arg(short = 'l', default_value = "eng")]
    language: String,
    /// Explicit .traineddata model file; needs its own read grant.
    #[arg(long)]
    model: Option<PathBuf>,
    /// Page segmentation mode: 3,4,5,6,7,8,9,10,11,13 (OSD excluded).
    #[arg(long, default_value = "3")]
    psm: i32,
    /// Only the LSTM engine is included.
    #[arg(long,default_value="1",value_parser=clap::value_parser!(u8).range(1..=1))]
    oem: u8,
    /// One output config: txt, tsv, hocr. External config files are unsupported.
    #[arg(default_value="txt",value_parser=["txt","tsv","hocr"])]
    config: String,
}

fn limited(reader: impl Read, label: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(INPUT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{label}: {e}"))?;
    if bytes.len() > INPUT_LIMIT {
        return Err(format!("{label}: exceeds 32 MiB input limit"));
    }
    Ok(bytes)
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::var_os("PWD")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(path)
    }
}

fn execute(cli: Cli) -> Result<(), String> {
    if ![3, 4, 5, 6, 7, 8, 9, 10, 11, 13].contains(&cli.psm) {
        return Err("unsupported --psm; OSD and segmentation-only modes are excluded".into());
    }
    if cli.language.is_empty()
        || cli.language.len() > 64
        || !cli
            .language
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("language must be a single ASCII identifier".into());
    }
    if cli.language != "eng" && cli.model.is_none() {
        return Err("only eng is embedded; supply --model for another language".into());
    }
    let bytes = if cli.input == "stdin" || cli.input == "-" {
        limited(io::stdin().lock(), "stdin")?
    } else {
        let path = absolute(Path::new(&cli.input));
        limited(
            File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?,
            &cli.input,
        )?
    };
    let mut reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12000);
    limits.max_image_height = Some(12000);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| format!("image header: {e}"))?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > PIXEL_LIMIT {
        return Err("image exceeds 20 million pixels or has zero dimensions".into());
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12000);
    limits.max_image_height = Some(12000);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let gray: Vec<u8> = {
        let pixels = reader
            .decode()
            .map_err(|e| format!("image decode: {e}"))?
            .to_luma_alpha8();
        pixels
            .pixels()
            .map(|p| {
                let alpha = u32::from(p[1]);
                ((u32::from(p[0]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
            })
            .collect()
    };
    let external;
    let model = if let Some(path) = cli.model {
        let path = absolute(&path);
        external = limited(
            File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?,
            "model",
        )?;
        &external
    } else {
        MODEL
    };
    let language = CString::new(cli.language).unwrap();
    let format = match cli.config.as_str() {
        "tsv" => 1,
        "hocr" => 2,
        _ => 0,
    };
    let mut output = std::ptr::null_mut();
    // C++ receives live buffers, validated dimensions and bounded model length.
    let status = unsafe {
        sat_ocr(
            gray.as_ptr(),
            width,
            height,
            model.as_ptr(),
            model.len() as i32,
            language.as_ptr(),
            cli.psm,
            format,
            &mut output,
        )
    };
    if status != 0 || output.is_null() {
        return Err(format!("Tesseract recognition failed (status {status})"));
    }
    let mut text = unsafe { CStr::from_ptr(output).to_bytes().to_vec() };
    unsafe { sat_ocr_free(output) };
    if cli.config == "tsv" {
        let mut document = b"level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n".to_vec();
        document.extend_from_slice(&text);
        text = document;
    } else if cli.config == "hocr" {
        let mut document = b"<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><meta charset=\"utf-8\"/><meta name=\"ocr-system\" content=\"tesseract 5.5.4\"/></head><body>\n".to_vec();
        document.extend_from_slice(&text);
        document.extend_from_slice(b"</body></html>\n");
        text = document;
    }
    if text.len() > 16 * 1024 * 1024 {
        return Err("OCR output exceeds 16 MiB".into());
    }
    if cli.outputbase == "stdout" || cli.outputbase == "-" {
        io::stdout()
            .lock()
            .write_all(&text)
            .map_err(|e| e.to_string())?;
    } else {
        let path = absolute(Path::new(&format!("{}.{}", cli.outputbase, cli.config)));
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .and_then(|mut f| f.write_all(&text))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

pub fn run_cli(args: Vec<String>) -> i32 {
    let cli = match Cli::try_parse_from(std::iter::once("tesseract".to_string()).chain(args)) {
        Ok(cli) => cli,
        Err(e) => {
            let code = e.exit_code();
            let _ = e.print();
            return code;
        }
    };
    match execute(cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("tesseract: {e}");
            1
        }
    }
}
