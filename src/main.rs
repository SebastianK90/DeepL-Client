#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod capture;
mod clipboard;
mod core;
mod document;
mod embedded_pdfium;
mod export_origin;
mod hotkey;
mod lookup;
mod provider;
mod replacement;
mod share;
mod speech;
mod ui;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--pdf-worker") {
        let result = document::pdf_worker::child_entry();
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    if std::env::args().nth(1).as_deref() == Some("--verify-pdf-worker") {
        let result = document::pdf_worker::self_test();
        if let Err(error) = &result {
            eprintln!("{error:#}");
        }
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    if std::env::args().nth(1).as_deref() == Some("--verify-export-origin") {
        let result = export_origin::self_test();
        if let Err(error) = &result {
            eprintln!("{error:#}");
        }
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    if std::env::args().nth(1).as_deref() == Some("--verify-pdf-engine") {
        let result = (|| -> anyhow::Result<()> {
            let pdfium = embedded_pdfium::library()?;
            let mut document = pdfium.create_new_pdf()?;
            document
                .pages_mut()
                .create_page_at_end(pdfium_render::prelude::PdfPagePaperSize::a4())?;
            let bytes = document.save_to_bytes()?;
            let read = pdfium.load_pdf_from_byte_vec(bytes, None)?;
            anyhow::ensure!(read.pages().len() == 1, "PDF self-check failed");
            Ok(())
        })();
        if let Err(error) = &result {
            eprintln!("{error:#}");
        }
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    if let Err(error) = ui::run() {
        eprintln!("DeepL Client konnte nicht gestartet werden: {error:#}");
        #[cfg(target_os = "windows")]
        {
            let message: Vec<u16> =
                format!("DeepL Client konnte nicht gestartet werden:\n\n{error:#}")
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
            let title: Vec<u16> = "DeepL Client"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    std::ptr::null_mut(),
                    message.as_ptr(),
                    title.as_ptr(),
                    0x10,
                );
            }
        }
        std::process::exit(1);
    }
}
