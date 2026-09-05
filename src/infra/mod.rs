//! 文件职责：基础设施层模块入口：JSON profile 读取、键值解析、进程探测、Zip 备份与监控实现。
//! 定义范围：模块导出；本层只被服务层依赖，反向依赖领域层抽象。

/// 版本 profile json 读取与加载器识别。
pub mod json_profile;
/// options 类 key:value 文本解析器与白名单合并策略。
pub mod key_value;
/// 运行中 java 进程枚举与实例目录关联判定。
pub mod process_probe;
/// Zip 镜像备份的打包与还原实现。
pub mod zip_archive;
/// Windows 无边框窗口镶边：命中测试子类化与 DWM 打磨（仅 Windows 编译）。
#[cfg(windows)]
pub mod window_chrome;
/// versions/ 目录监控与稳定性判定的默认实现。
pub mod watcher;
