//! Build one optimized LGB1 image for `logos-jit-run`.

#[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
fn main() {
    native::main();
}

#[cfg(not(all(target_arch = "x86_64", not(target_os = "windows"))))]
fn main() {
    eprintln!("logos-jit-image-build requires x86-64 on a non-Windows platform");
    std::process::exit(2);
}

#[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
mod native {
    use std::path::{Path, PathBuf};

    pub(super) fn main() {
        let args: Vec<String> = std::env::args().collect();
        let result = match args.as_slice() {
            [_, source, image] => build(Path::new(source), Path::new(image)),
            _ => Err("usage: logos-jit-image-build SOURCE IMAGE".into()),
        };
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }

    fn build(source_path: &Path, image_path: &Path) -> Result<(), String> {
        let source = std::fs::read_to_string(source_path).map_err(|e| e.to_string())?;
        let bytes = logicaffeine_jit::image::build(&source)?;
        let temporary = temporary_path(image_path);
        std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, image_path).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            e.to_string()
        })
    }

    fn temporary_path(image_path: &Path) -> PathBuf {
        let mut name = image_path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_else(|| "program.lgb".into());
        name.push(format!(".{}.tmp", std::process::id()));
        image_path.with_file_name(name)
    }
}
