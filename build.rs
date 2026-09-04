//! 构建脚本：编译 Slint UI 描述文件并生成 Rust 绑定。
fn main() {
    slint_build::compile("ui/packporter.slint").expect("Slint UI 编译失败");
}
