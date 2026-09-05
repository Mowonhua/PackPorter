//! 构建脚本：编译 Slint UI，并将 Windows 图标资源链接进可执行文件。
fn main() {
    println!("cargo:rerun-if-changed=assets/icons/packporter.png");
    println!("cargo:rerun-if-changed=assets/icons/packporter.ico");
    slint_build::compile("ui/packporter.slint").expect("Slint UI 编译失败");

    // 按目标平台判断，保证交叉编译到 Windows 时也包含资源；失败不可降级为无图标产物。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/packporter.ico")
            .compile()
            .expect("Windows 图标资源编译失败");
    }
}
