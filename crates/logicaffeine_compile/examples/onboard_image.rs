//! onboard_image — the board's boot image and the files its firmware and RTL are built against, all from
//! `codegen_sva::hart::{soc, image}`:
//!
//!   onboard_image header <dir>                    write <dir>/soc.h (the firmware's C) and <dir>/soc_defines.vh (the RTL)
//!   onboard_image rom-hex <boot.bin> <out.hex>    the boot ROM as the $readmemh words its block RAM starts with
//!   onboard_image emit bringup <dir> <boot.bin>   a bitstream's sources, and the TOP / SRC / XDC / PRE lines
//!   onboard_image emit loopback <dir>             scripts/openxc7-build.sh takes (paths from benchmarks/silicon):
//!                                                 the bring-up image (with its boot ROM and its fingerprint in
//!                                                 <dir>/fingerprint.txt), or the DDR2 PHY's loopback probe
//!   onboard_image pack <out.img> --rtl-md5 <hex32> --shim <shim.bin> --firmware <elf> --bundle <dir> [-- args…]
//!                                                 a boot image: the bundle holds every file under <dir>, by
//!                                                 /library/<path>; the arguments after `--` are the compiler's
//!   onboard_image inspect <img>                   check an image as the boot ROM does and list its sections

use logicaffeine_compile::codegen_sva::hart::{image, rtl, soc};

fn fail(why: &str) -> ! {
    eprintln!("onboard_image: {why}");
    std::process::exit(2);
}

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| fail(&format!("cannot read {path}: {e}")))
}

fn files_under(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| fail(&format!("cannot read {}: {e}", dir.display())))
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files_under(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push((format!("/library/{rel}"), std::fs::read(&path).unwrap()));
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("header") => {
            let dir = std::path::Path::new(args.get(1).unwrap_or_else(|| fail("header <dir>")));
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("soc.h"), soc::c_header()).unwrap();
            std::fs::write(dir.join(rtl::DEFINES), soc::verilog_defines()).unwrap();
        }
        Some("rom-hex") => {
            let (bin, out) = (args.get(1), args.get(2));
            let (Some(bin), Some(out)) = (bin, out) else { fail("rom-hex <boot.bin> <out.hex>") };
            let text = rtl::rom_hex(&read(bin)).unwrap_or_else(|e| fail(&e));
            std::fs::write(out, text).unwrap();
        }
        Some("emit") => {
            let (Some(which), Some(dir)) = (args.get(1), args.get(2)) else { fail("emit bringup|loopback <dir> …") };
            let (top, files, rom) = match which.as_str() {
                "bringup" => {
                    let boot = args.get(3).unwrap_or_else(|| fail("emit bringup <dir> <boot.bin>"));
                    let rom = rtl::rom_hex(&read(boot)).unwrap_or_else(|e| fail(&e));
                    ("hart_bringup_top", rtl::bringup_files(), Some(rom))
                }
                "loopback" => ("hart_ddr2_loopback_top", rtl::loopback_files(), None),
                other => fail(&format!("no image {other}: bringup or loopback")),
            };
            let out = std::path::Path::new(dir);
            std::fs::create_dir_all(out).unwrap();
            for (name, text) in &files {
                std::fs::write(out.join(name), text).unwrap();
            }
            let silicon = std::fs::canonicalize("benchmarks/silicon")
                .unwrap_or_else(|e| fail(&format!("run from the repository root: {e}")));
            let rel = std::fs::canonicalize(out)
                .unwrap()
                .strip_prefix(&silicon)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| fail("<dir> must be under benchmarks/silicon"));
            let rel = rel.display();
            let src: Vec<String> =
                files.iter().filter(|(n, _)| n.ends_with(".v")).map(|(n, _)| format!("{rel}/{n}")).collect();
            println!("TOP={top}");
            println!("SRC=\"{}\"", src.join(" "));
            println!("XDC={rel}/{top}.xdc");
            if let Some(rom) = rom {
                std::fs::write(out.join("boot_rom.hex"), &rom).unwrap();
                let print = rtl::fingerprint(&files, &rom);
                let hex: String = print.iter().map(|b| format!("{b:02x}")).collect();
                std::fs::write(out.join("fingerprint.txt"), format!("{hex}\n")).unwrap();
                let (lo, hi) = rtl::fingerprint_words(print);
                println!("fingerprint={hex}");
                println!(
                    "PRE=\"chparam -set RTL_MD5_LO 64'h{lo:016x} -set RTL_MD5_HI 64'h{hi:016x} \
                     -set ROM_INIT \\\"{rel}/boot_rom.hex\\\" {top};\""
                );
            }
        }
        Some("pack") => {
            let out = args.get(1).unwrap_or_else(|| fail("pack <out.img> …"));
            let mut rtl_md5 = None;
            let (mut shim, mut firmware, mut bundle_dir) = (None, None, None);
            let mut run_args = Vec::new();
            let mut it = args[2..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--rtl-md5" => rtl_md5 = it.next().cloned(),
                    "--shim" => shim = it.next().map(|p| read(p)),
                    "--firmware" => firmware = it.next().map(|p| read(p)),
                    "--bundle" => bundle_dir = it.next().cloned(),
                    "--" => run_args.extend(it.by_ref().cloned()),
                    other => fail(&format!("unknown argument {other}")),
                }
            }
            let md5_hex = rtl_md5.unwrap_or_else(|| fail("--rtl-md5 is required"));
            if md5_hex.len() != 32 {
                fail("--rtl-md5 takes 32 hex digits");
            }
            let mut md5 = [0u8; 16];
            for (i, b) in md5.iter_mut().enumerate() {
                *b = u8::from_str_radix(&md5_hex[2 * i..2 * i + 2], 16).unwrap_or_else(|_| fail("--rtl-md5 is not hex"));
            }
            let mut bundle = Vec::new();
            if let Some(dir) = bundle_dir {
                let root = std::path::Path::new(&dir);
                files_under(root, root, &mut bundle);
            }
            let img = image::build(&image::BootImage {
                rtl_md5: md5,
                shim: shim.unwrap_or_else(|| fail("--shim is required")),
                firmware: firmware.unwrap_or_else(|| fail("--firmware is required")),
                bundle,
                args: run_args,
            });
            if soc::IMAGE_OFFSET + img.len() as u64 > soc::FLASH_SIZE {
                fail(&format!("the image is {} bytes; the flash holds {} after the bitstream", img.len(),
                              soc::FLASH_SIZE - soc::IMAGE_OFFSET));
            }
            std::fs::write(out, &img).unwrap();
            println!("image={out} bytes={}", img.len());
        }
        Some("inspect") => {
            let img = read(args.get(1).unwrap_or_else(|| fail("inspect <img>")));
            let (md5, body) = image::open(&img).unwrap_or_else(|e| fail(&e));
            println!("rtl_md5={}", md5.iter().map(|b| format!("{b:02x}")).collect::<String>());
            println!("packed={} inflated={}", img.len() - soc::IMAGE_HEADER_SIZE as usize, body.len());
            let count = u32::from_le_bytes(body[0..4].try_into().unwrap()) as usize;
            for i in 0..count {
                let e = &body[8 + 24 * i..8 + 24 * (i + 1)];
                let kind = u32::from_le_bytes(e[0..4].try_into().unwrap());
                let length = u64::from_le_bytes(e[16..24].try_into().unwrap());
                println!("section kind={kind} bytes={length}");
            }
        }
        _ => fail("header | rom-hex | emit | pack | inspect"),
    }
}
