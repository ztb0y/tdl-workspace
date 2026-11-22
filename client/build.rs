use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
};

fn main() {
    slint_build::compile("ui/client.slint").expect("compile Slint UI");
    println!("cargo:rerun-if-changed=ui/client.slint");
    println!("cargo:rerun-if-changed=ui/telegram-logo.svg");
    println!("cargo:rerun-if-changed=ui/telegram-app-icon.svg");
    println!("cargo:rerun-if-changed=../tdl-web-ui/public/telegram-pattern.svg");
    println!("cargo:rerun-if-changed=bundle/runtime.zip");

    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_windows_icon();
    }
}

fn embed_windows_icon() {
    let svg = fs::read("ui/telegram-app-icon.svg").expect("read application icon");
    let ico = render_ico(&svg);
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    let ico_path = out_dir.join("telegram-app-icon.ico");
    fs::write(&ico_path, &ico).expect("write application icon");

    let rc_path = out_dir.join("telegram-app-icon.rc");
    let resource_path = out_dir.join("telegram-app-icon.res");
    let ico_path = ico_path.to_string_lossy().replace('\\', "/");
    fs::write(&rc_path, format!("1 ICON \"{ico_path}\"\n")).expect("write Windows icon script");
    let compiler = find_resource_compiler().expect("find Windows resource compiler");
    let status = Command::new(compiler)
        .args(["/nologo", "/fo"])
        .arg(&resource_path)
        .arg(&rc_path)
        .stdin(Stdio::null())
        .status()
        .expect("run Windows resource compiler");
    assert!(
        status.success(),
        "Windows resource compiler failed: {status}"
    );

    for binary in ["tdl-web-launcher", "tdl-web-setup", "tdl-web-uninstaller"] {
        println!(
            "cargo:rustc-link-arg-bin={binary}={}",
            resource_path.display()
        );
    }
}

fn find_resource_compiler() -> Option<PathBuf> {
    if let Some(path) = env::var_os("PATH").and_then(|value| {
        env::split_paths(&value)
            .map(|dir| dir.join("rc.exe"))
            .find(|path| path.is_file())
    }) {
        return Some(path);
    }

    if let Some(sdk_dir) = env::var_os("WindowsSdkDir") {
        if let Some(path) = find_resource_compiler_in(&PathBuf::from(sdk_dir).join("bin")) {
            return Some(path);
        }
    }

    find_resource_compiler_in(
        PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin").as_path(),
    )
}

fn find_resource_compiler_in(root: &std::path::Path) -> Option<PathBuf> {
    let mut versions = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    versions.sort();
    versions.reverse();
    versions
        .into_iter()
        .map(|version| version.join("x64").join("rc.exe"))
        .find(|path| path.is_file())
}

fn render_ico(svg: &[u8]) -> Vec<u8> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .expect("parse application icon SVG");
    let source_size = tree.size();
    let sizes = [16_u32, 24, 32, 48, 64, 128, 256];
    let images = sizes
        .iter()
        .map(|size| {
            let mut pixmap =
                resvg::tiny_skia::Pixmap::new(*size, *size).expect("allocate application icon");
            let scale = *size as f32 / source_size.width().max(source_size.height());
            let offset_x = (*size as f32 - source_size.width() * scale) / 2.0;
            let offset_y = (*size as f32 - source_size.height() * scale) / 2.0;
            let transform = resvg::tiny_skia::Transform::from_scale(scale, scale)
                .post_translate(offset_x, offset_y);
            resvg::render(&tree, transform, &mut pixmap.as_mut());
            pixmap.encode_png().expect("encode application icon")
        })
        .collect::<Vec<_>>();

    let header_size = 6 + 16 * images.len();
    let data_size = images.iter().map(Vec::len).sum::<usize>();
    let mut ico = Vec::with_capacity(header_size + data_size);
    push_u16(&mut ico, 0);
    push_u16(&mut ico, 1);
    push_u16(&mut ico, images.len() as u16);
    let mut offset = header_size as u32;
    for (size, image) in sizes.iter().zip(&images) {
        ico.push(if *size == 256 { 0 } else { *size as u8 });
        ico.push(if *size == 256 { 0 } else { *size as u8 });
        ico.push(0);
        ico.push(0);
        push_u16(&mut ico, 1);
        push_u16(&mut ico, 32);
        push_u32(&mut ico, image.len() as u32);
        push_u32(&mut ico, offset);
        offset += image.len() as u32;
    }
    for image in images {
        ico.extend_from_slice(&image);
    }
    ico
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}
