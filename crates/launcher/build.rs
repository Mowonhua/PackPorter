//! 文件职责：将应用图标嵌入独立 shim。
//! 定义范围：Windows 资源编译；不编译 Slint UI。

fn main() {
    let icon = "../../assets/icons/packporter.ico";
    println!("cargo:rerun-if-changed={icon}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon(icon)
            .compile()
            .expect("Windows shim 图标资源编译失败");
    }
}
