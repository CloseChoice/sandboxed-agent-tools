use image::ImageDecoder;
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use std::path::{Path, PathBuf};
use wit_bindgen::generate;

generate!({ world: "pdf-tools" });

const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_INPUT_FILES: usize = 256;
const MAX_TOTAL_PIXELS: u64 = 4_000_000;

struct PdfTools;

impl Guest for PdfTools {
    fn run(convert: Option<Vec<String>>, merge: Option<String>) -> Result<String, String> {
        if let Ok(pwd) = std::env::var("PWD") {
            std::env::set_current_dir(pwd)
                .map_err(|e| format!("could not select granted working directory: {}", e.kind()))?;
        }
        let (bytes, pages) = match (convert, merge) {
            (Some(inputs), None) => convert_images(&inputs)?,
            (None, Some(input)) => merge_pdfs(Path::new(&input))?,
            _ => return Err("choose exactly one of --convert or --merge".into()),
        };
        destination::write(&bytes)
            .map_err(|_| "could not write PDF to the granted destination".to_owned())?;
        Ok(format!("Wrote PDF ({pages} pages)"))
    }
}

export!(PdfTools);

fn supported_image(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("png")
            || extension.eq_ignore_ascii_case("jpg")
            || extension.eq_ignore_ascii_case("jpeg")
    })
}

fn files_in(path: &Path, predicate: impl Fn(&Path) -> bool) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return if predicate(path) {
            Ok(vec![path.to_path_buf()])
        } else {
            Err("input file has an unsupported extension".into())
        };
    }
    if !path.is_dir() {
        return Err("input must be a granted file or directory".into());
    }
    let mut files = std::fs::read_dir(path)
        .map_err(|e| format!("could not list input directory: {}", e.kind()))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| format!("could not list input directory: {}", e.kind()))?;
    files.retain(|p| p.is_file() && predicate(p));
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    if files.is_empty() {
        return Err("input directory contains no supported files".into());
    }
    if files.len() > MAX_INPUT_FILES {
        return Err(format!(
            "input directory exceeds the {MAX_INPUT_FILES}-file limit"
        ));
    }
    Ok(files)
}

fn read_limited(path: &Path) -> Result<Vec<u8>, String> {
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("could not read input: {}", e.kind()))?;
    if metadata.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "input exceeds the {} MiB file limit",
            MAX_INPUT_BYTES / 1024 / 1024
        ));
    }
    std::fs::read(path).map_err(|e| format!("could not read input: {}", e.kind()))
}

fn convert_images(inputs: &[String]) -> Result<(Vec<u8>, usize), String> {
    let mut paths = Vec::new();
    for input in inputs {
        paths.extend(files_in(Path::new(input), supported_image)?);
        if paths.len() > MAX_INPUT_FILES {
            return Err(format!("inputs exceed the {MAX_INPUT_FILES}-file limit"));
        }
    }
    if paths.is_empty() {
        return Err("provide at least one image to convert".into());
    }
    let pages_id = (1, 0);
    let mut doc = Document::with_version("1.5");
    let mut page_ids = Vec::with_capacity(paths.len());
    let mut total_pixels = 0_u64;
    let mut total_input_bytes = 0_u64;

    for path in paths {
        let encoded = read_limited(&path)?;
        total_input_bytes += encoded.len() as u64;
        if total_input_bytes > MAX_INPUT_BYTES {
            return Err("input images exceed the 16 MiB total per-operation limit".into());
        }
        let dimensions = image::ImageReader::new(std::io::Cursor::new(encoded.as_slice()))
            .with_guessed_format()
            .map_err(|_| "could not identify image format")?
            .into_dimensions()
            .map_err(|_| "could not read image dimensions")?;
        let (image_width, image_height) = dimensions;
        if image_width == 0
            || image_height == 0
            || u64::from(image_width) * u64::from(image_height) > 5_000_000
        {
            return Err("image dimensions are outside the supported range".into());
        }
        let width = i64::from(image_width);
        let height = i64::from(image_height);
        let is_jpeg = path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg")
        });
        let (image_bytes, color_space) = if is_jpeg {
            // Keep JPEGs in their compressed form. Decoding a photo in WASI uses
            // far more fuel than preserving its DCT stream in the PDF.
            let decoder = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(&encoded))
                .map_err(|_| "could not read JPEG headers")?;
            let color_space = match decoder.color_type() {
                image::ColorType::Rgb8 => "DeviceRGB",
                image::ColorType::L8 => "DeviceGray",
                _ => return Err("only grayscale and RGB JPEG images are supported".into()),
            };
            (encoded, color_space)
        } else {
            total_pixels += u64::from(image_width) * u64::from(image_height);
            if total_pixels > MAX_TOTAL_PIXELS {
                return Err("PNG images exceed the 4-megapixel total per-operation limit".into());
            }
            let decoded = image::load_from_memory(&encoded).map_err(|_| {
                format!(
                    "could not decode image {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )
            })?;

            // Flatten transparency onto white; PDF image masks are outside this MVP.
            let rgba = decoded.into_rgba8().into_raw();
            let mut rgb = Vec::with_capacity((width * height * 3) as usize);
            for pixel in rgba.chunks_exact(4) {
                let alpha = u16::from(pixel[3]);
                for channel in &pixel[..3] {
                    rgb.push(((u16::from(*channel) * alpha + 255 * (255 - alpha)) / 255) as u8);
                }
            }
            (rgb, "DeviceRGB")
        };
        let mut image_dict = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => width,
            "Height" => height,
            "ColorSpace" => color_space,
            "BitsPerComponent" => 8,
        };
        if is_jpeg {
            image_dict.set("Filter", "DCTDecode");
        }
        let image = Stream::new(image_dict, image_bytes);
        let image_id = doc.add_object(image);
        let image_name = format!("Im{}", image_id.0);
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        width.into(),
                        0.into(),
                        0.into(),
                        height.into(),
                        0.into(),
                        0.into(),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(image_name.as_bytes().to_vec())]),
                Operation::new("Q", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            content.encode().map_err(|_| "could not encode PDF page")?,
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(pages_id),
            "Contents" => Object::Reference(content_id),
            "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
        });
        doc.add_xobject(page_id, image_name.as_bytes(), image_id)
            .map_err(|_| "could not add image to PDF page")?;
        page_ids.push(page_id);
    }

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_ids.len() as i64,
        }),
    );
    let catalog_id =
        doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => Object::Reference(pages_id) });
    doc.trailer.set("Root", Object::Reference(catalog_id));
    let mut result = Vec::new();
    doc.save_to(&mut result)
        .map_err(|_| "could not serialize PDF")?;
    Ok((result, page_ids.len()))
}

fn merge_pdfs(input: &Path) -> Result<(Vec<u8>, usize), String> {
    let paths = files_in(input, |p| {
        p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
    })?;
    let mut docs = Vec::with_capacity(paths.len());
    let mut total_input_bytes = 0_u64;
    for path in paths {
        let bytes = read_limited(&path)?;
        total_input_bytes += bytes.len() as u64;
        if total_input_bytes > MAX_INPUT_BYTES {
            return Err("input PDFs exceed the 16 MiB total per-operation limit".into());
        }
        let doc = Document::load_mem_with_options(
            &bytes,
            lopdf::LoadOptions {
                max_decompressed_size: Some(8 * 1024 * 1024),
                ..lopdf::LoadOptions::default()
            },
        )
        .map_err(|_| {
            format!(
                "could not parse PDF {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
        })?;
        if doc.is_encrypted() {
            return Err("encrypted PDFs are not supported".into());
        }
        docs.push(doc);
    }
    let page_count = docs.iter().map(|doc| doc.get_pages().len()).sum();
    let mut merged = merge_documents(docs)?;
    let mut result = Vec::new();
    merged
        .save_to(&mut result)
        .map_err(|_| "could not serialize merged PDF")?;
    Ok((result, page_count))
}

// Combine page trees and their referenced objects. This preserves page content
// and page resources; document-level outlines, forms and named destinations
// are not carried across.
fn merge_documents(docs: Vec<Document>) -> Result<Document, String> {
    use std::collections::BTreeMap;
    let mut max_id = 1;
    let mut pages = BTreeMap::<ObjectId, Object>::new();
    let mut objects = BTreeMap::<ObjectId, Object>::new();
    let mut output = Document::with_version("1.5");
    let mut catalog = None;
    let mut page_tree = None;
    let mut page_tree_id = None;

    for mut doc in docs {
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;
        let root_pages_id = doc
            .objects
            .iter()
            .find_map(|(id, object)| {
                (object.type_name().ok() == Some(b"Pages".as_slice())
                    && object.as_dict().ok()?.get(b"Parent").is_err())
                .then_some(*id)
            })
            .ok_or("input PDF has no root page tree")?;
        let root_pages = doc
            .get_dictionary(root_pages_id)
            .map_err(|_| "invalid PDF page tree")?
            .clone();
        let mut inherited = Vec::new();
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            if let Ok(value) = root_pages.get(key) {
                inherited.push((key.to_vec(), value.clone()));
            }
        }
        for (id, object) in &doc.objects {
            match object.type_name().unwrap_or(b"") {
                b"Catalog" if catalog.is_none() => catalog = Some((*id, object.clone())),
                b"Pages" if *id == root_pages_id && page_tree.is_none() => {
                    page_tree_id = Some(*id);
                    page_tree = Some((*id, object.clone()));
                }
                b"Page" => {
                    let mut page = object.as_dict().map_err(|_| "invalid PDF page")?.clone();
                    for (key, value) in &inherited {
                        if !page.has(key) {
                            page.set(key.clone(), value.clone());
                        }
                    }
                    page.set(
                        "Parent",
                        Object::Reference(page_tree_id.unwrap_or(root_pages_id)),
                    );
                    pages.insert(*id, Object::Dictionary(page));
                }
                b"Outlines" | b"Outline" | b"Pages" | b"Catalog" => {}
                _ => {
                    objects.insert(*id, object.clone());
                }
            }
        }
    }
    let (catalog_id, catalog_obj) = catalog.ok_or("input PDF has no catalog")?;
    let (pages_id, pages_obj) = page_tree.ok_or("input PDF has no page tree")?;
    output.objects.extend(objects);
    output.objects.extend(pages.clone());
    let mut pages_dict = pages_obj
        .as_dict()
        .map_err(|_| "invalid PDF page tree")?
        .clone();
    pages_dict.set(
        "Kids",
        pages
            .keys()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    pages_dict.set("Count", pages.len() as i64);
    output
        .objects
        .insert(pages_id, Object::Dictionary(pages_dict));
    let mut catalog_dict = catalog_obj
        .as_dict()
        .map_err(|_| "invalid PDF catalog")?
        .clone();
    catalog_dict.set("Pages", Object::Reference(pages_id));
    catalog_dict.remove(b"Outlines");
    output
        .objects
        .insert(catalog_id, Object::Dictionary(catalog_dict));
    output.trailer.set("Root", Object::Reference(catalog_id));
    output.max_id = output.objects.keys().map(|id| id.0).max().unwrap_or(0);
    output.renumber_objects();
    output.adjust_zero_pages();
    Ok(output)
}
