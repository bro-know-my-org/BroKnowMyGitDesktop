//! Native asset/decoder smoke test (requires a desktop display and `pnpm dev`).
//! Run: cargo run --manifest-path src-tauri/Cargo.toml --example resource_native_smoke
//! Uses an isolated temporary Git repository and never opens the user's app store.
use broknowmygitdesktop_lib::commands;
use std::sync::atomic::{AtomicI32, Ordering};
use tauri::Manager;

static EXIT_CODE: AtomicI32 = AtomicI32::new(2);

#[tauri::command]
fn report_native_resource(result: String, app: tauri::AppHandle) {
    println!("NATIVE_RESOURCE_RESULT={result}");
    let success = serde_json::from_str::<Vec<serde_json::Value>>(&result).is_ok_and(|rows| {
        rows.len() == 4
            && ["test.png", "test.svg", "test.wav", "test.pdf"]
                .iter()
                .all(|path| {
                    rows.iter().any(|row| {
                        row["path"] == *path
                            && row["status"] == 200
                            && (row["pixelsReadable"] == true || row["componentDecoded"] == true)
                    })
                })
    });
    let code = if success { 0 } else { 1 };
    EXIT_CODE.store(code, Ordering::SeqCst);
    app.exit(code);
}
fn main() {
    let repository = tempfile::Builder::new()
        .prefix("bkm-resource-native-")
        .tempdir()
        .unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(repository.path())
        .status()
        .unwrap()
        .success());
    std::fs::write(
        repository.path().join("test.png"),
        include_bytes!("../icons/32x32.png"),
    )
    .unwrap();
    std::fs::write(repository.path().join("test.svg"), r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##).unwrap();
    let mut wav = vec![0u8; 16044];
    for (offset, bytes) in [(0, b"RIFF"), (8, b"WAVE"), (12, b"fmt "), (36, b"data")] {
        wav[offset..offset + 4].copy_from_slice(bytes);
    }
    for (offset, value) in [
        (4, 16036u32),
        (16, 16),
        (24, 8000),
        (28, 16000),
        (40, 16000),
    ] {
        wav[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [(20, 1u16), (22, 1), (32, 2), (34, 16)] {
        wav[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    std::fs::write(repository.path().join("test.wav"), wav).unwrap();
    let stream = "1 0 0 rg 0 0 300 200 re f";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R >>".into(),
        format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ),
    ];
    let mut pdf = "%PDF-1.4\n".to_owned();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf += &format!("{} 0 obj\n{object}\nendobj\n", i + 1);
    }
    let xref = pdf.len();
    pdf += "xref\n0 5\n0000000000 65535 f \n";
    for offset in offsets {
        pdf += &format!("{offset:010} 00000 n \n");
    }
    pdf += &format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF");
    std::fs::write(repository.path().join("test.pdf"), pdf).unwrap();
    let path = repository.path().to_string_lossy().into_owned();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::get_resource_preview,
            commands::release_resource_preview,
            report_native_resource
        ])
        .setup(move |app| {
            let state = commands::ResourcePreviews::new()?;
            app.asset_protocol_scope()
                .allow_directory(state.path(), true)?;
            app.manage(state);
            let script = format!(
                "window.resourceTestRepository = {};",
                serde_json::to_string(&path).unwrap()
            );
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(
                    "http://localhost:51420/scripts/fixtures/resource-native-smoke.html"
                        .parse()
                        .unwrap(),
                ),
            )
            .initialization_script(&script)
            .visible(false)
            .build()?;
            app.manage(repository);
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(30));
                eprintln!("Native resource smoke test timed out");
                handle.exit(2);
            });
            Ok(())
        })
        .run(context)
        .unwrap();
    std::process::exit(EXIT_CODE.load(Ordering::SeqCst));
}
