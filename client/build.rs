fn main() {
    slint_build::compile("ui/client.slint").expect("compile Slint UI");
    println!("cargo:rerun-if-changed=ui/client.slint");
    println!("cargo:rerun-if-changed=bundle/runtime.zip");
}
