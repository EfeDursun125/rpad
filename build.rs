fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("material".into()),
    )
    .expect("compile rpad UI");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resources = winresource::WindowsResource::new();
        resources.set_icon("assets/rpad.ico");
        resources.set("ProductName", "RPad");
        resources.set("FileDescription", "RPad text editor");
        if cfg!(not(windows)) {
            resources.set_toolkit_path("/usr/bin");
        }
        resources.compile().expect("embed Windows application icon");
    }
    println!("cargo:rerun-if-changed=assets/rpad.ico");
}
