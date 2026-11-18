use lopdf::{Document, LoadOptions};
use serde::Serialize;
use std::{fs::File, io::Read};

wit_bindgen::generate!({ world: "pdf-text" });

const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DECOMPRESSED_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAGE_CONTENT_BYTES: usize = 2 * 1024 * 1024;
const MAX_DOCUMENT_PAGES: usize = 10_000;

struct PdfText;

impl Guest for PdfText {
    fn extract(
        path: String,
        start_page: u32,
        max_pages: u32,
        max_bytes: u32,
        format: String,
    ) -> Result<String, String> {
        if start_page == 0 {
            return Err("start-page must be at least 1".into());
        }
        if !(1..=200).contains(&max_pages) {
            return Err("max-pages must be between 1 and 200".into());
        }
        if !(1..=1_048_576).contains(&max_bytes) {
            return Err("max-bytes must be between 1 and 1048576".into());
        }
        if !matches!(format.as_str(), "text" | "json") {
            return Err("format must be text or json".into());
        }
        // Bound the actual read rather than relying on metadata that can change.
        let file = File::open(path).map_err(|_| "could not open the granted PDF input")?;
        let mut bytes = Vec::new();
        file.take(MAX_INPUT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "could not read the granted PDF input")?;
        if bytes.len() as u64 > MAX_INPUT_BYTES {
            return Err("PDF input exceeds the 16 MiB file limit".into());
        }
        let doc = Document::load_mem_with_options(
            &bytes,
            LoadOptions {
                max_decompressed_size: Some(MAX_DECOMPRESSED_BYTES),
                ..LoadOptions::default()
            },
        )
        .map_err(|_| "could not parse PDF (malformed, encrypted, or unsupported input)")?;
        if doc.is_encrypted() || doc.was_encrypted() {
            return Err("encrypted PDFs are not supported".into());
        }
        let total_pages = doc.page_iter().take(MAX_DOCUMENT_PAGES + 1).count();
        if total_pages == 0 || total_pages > MAX_DOCUMENT_PAGES {
            return Err("PDF must contain between 1 and 10000 pages".into());
        }
        let total_pages = total_pages as u32;
        if start_page > total_pages {
            return Err(format!("start-page exceeds the PDF's {total_pages} pages"));
        }

        let last_page = total_pages.min(start_page.saturating_add(max_pages - 1));
        let mut remaining = max_bytes as usize;
        let mut pages = Vec::new();
        let mut next_page = (last_page < total_pages).then_some(last_page + 1);
        for number in start_page..=last_page {
            // This also bounds font-map and page-content decompression.
            let mut text = doc
                .extract_text_with_limit(&[number], MAX_PAGE_CONTENT_BYTES)
                .map_err(|_| format!("could not extract page {number} (unsupported content or decompression limit)"))?;
            if remaining == 0 && !text.is_empty() {
                next_page = Some(number);
                break;
            }
            let truncated = text.len() > remaining;
            if truncated {
                let mut boundary = remaining;
                while !text.is_char_boundary(boundary) {
                    boundary -= 1;
                }
                text.truncate(boundary);
            }
            remaining -= text.len();
            pages.push(Page {
                number,
                text,
                truncated,
            });
            if truncated {
                // Retry this page with a larger budget to retrieve its full text.
                next_page = Some(number);
                break;
            }
        }
        let extraction = Extraction {
            total_pages,
            pages,
            next_page,
            truncated: next_page.is_some(),
        };
        if format == "json" {
            serde_json::to_string(&extraction).map_err(|_| "could not encode extracted text".into())
        } else {
            Ok(extraction.render())
        }
    }
}

#[derive(Serialize)]
struct Page {
    number: u32,
    text: String,
    truncated: bool,
}

#[derive(Serialize)]
struct Extraction {
    total_pages: u32,
    pages: Vec<Page>,
    next_page: Option<u32>,
    truncated: bool,
}

impl Extraction {
    fn render(&self) -> String {
        let mut output = String::new();
        for page in &self.pages {
            output.push_str(&format!(
                "--- Page {} of {} ---\n",
                page.number, self.total_pages
            ));
            if page.text.trim().is_empty() && !page.truncated {
                output.push_str("[No extractable embedded text; OCR may be needed.]\n");
            } else {
                output.push_str(&page.text);
                if !page.text.ends_with('\n') {
                    output.push('\n');
                }
            }
            if page.truncated {
                output.push_str("[Page text clipped by max-bytes; increase the budget to read the full page.]\n");
            }
        }
        if let Some(next) = self.next_page {
            output.push_str(&format!("[More text available: --start-page {next}; increase --max-bytes if this page was clipped.]\n"));
        }
        output
    }
}

export!(PdfText);
