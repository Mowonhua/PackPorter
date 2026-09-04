//! 文件职责：实现 VersionProfileReader：解析版本 json（含 inheritsFrom 继承链）与加载器识别。
//! 定义范围：JsonProfileReader 及其中间解析结构；不含目录枚举（属于 InstanceService）。

use std::path::Path;

use serde::Deserialize;

use crate::domain::error::{PackError, PackResult};
use crate::domain::instance::{InstanceProfile, LoaderKind, MinecraftVersion, VersionProfileReader};

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：版本 json 中与本工具相关的字段子集（serde 只取所需，忽略其余）。
 * 字段说明：继承链解析时子 profile 的空缺字段由父 profile 补齐。
 * 约束条件：id 可缺省（PCL2 导出格式不强制）；其余字段均可缺省。
 */
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RawVersionProfile {
    /// 版本 id，通常等于目录名。
    #[serde(default)]
    pub id: String,
    /// 继承的父版本 id（整合包常见，如 vanilla 链）。
    #[serde(default)]
    pub inherits_from: Option<String>,
    /// MC 基础版本号：优先 minecraft_version，其次 client_version。
    #[serde(default)]
    pub minecraft_version: Option<String>,
    /// 原版版本号字段（PCL2/HMCL 整合包 json 常见，等价于 MC 版本）。
    #[serde(default, rename = "clientVersion")]
    pub client_version: Option<String>,
    /// 依赖库清单：用于加载器关键字识别。
    #[serde(default)]
    pub libraries: Vec<RawLibrary>,
    /// 主类：forge/fabric 的特征判据之一。
    #[serde(default)]
    pub main_class: Option<String>,
}

/**
 * 结构职责：依赖库条目的最小字段子集。
 * 字段说明：name 形如 "group:artifact:version"，artifact 段用于加载器识别。
 * 约束条件：只读 name 字段。
 */
#[derive(Debug, Clone, Deserialize)]
pub struct RawLibrary {
    /// 库坐标字符串。
    pub name: String,
}

// ==================== 函数和方法定义 ====================

impl JsonProfileReader {
    /**
     * 函数职责：从单份 json 文本解析出（id, mc_version, loader 线索集）。
     * 输入说明：raw 为 json 文本。
     * 输出说明：解析失败返回 JsonParse。
     * 实现思路：serde 反序列化，缺省字段走 Default。
     */
    fn parse_raw(raw: &str) -> PackResult<RawVersionProfile> {
        serde_json::from_str(raw).map_err(|e| PackError::JsonParse {
            path: String::new(),
            message: e.to_string(),
        })
    }

    /**
     * 函数职责：沿 inheritsFrom 链递归读取，合并出最完整的元数据。
     * 输入说明：dir 为版本目录；versions_root 用于解析父版本目录；seen 防继承环。
     * 输出说明：(mc_version, loader, loader_version) 合并结果；本目录无 json 时返回 Vanilla 兜底。
     * 实现思路：先递归父目录取父元数据 → 再读本目录 json → 子字段优先覆盖父字段；
     *           加载器识别综合 libraries 坐标、main_class 与整份 json 小写文本关键词。
     */
    pub fn resolve_inherited(
        dir: &Path,
        versions_root: &Path,
        seen: &mut Vec<String>,
    ) -> PackResult<(String, LoaderKind, Option<String>)> {
        // 目录名作为防环标识。
        let dir_key = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if seen.contains(&dir_key) {
            // 继承环：直接返回兜底，避免死循环。
            return Ok(("unknown".to_string(), LoaderKind::Vanilla, None));
        }
        seen.push(dir_key);

        // 定位目录内 json：优先 <目录名>.json，否则取目录内第一个 .json。
        let profile_path = Self::locate_profile(dir);
        let Some(profile_path) = profile_path else {
            // 无 json 的裸目录（如仅 jar）：按 Vanilla 兜底，不视为错误。
            return Ok(("unknown".to_string(), LoaderKind::Vanilla, None));
        };

        let raw = std::fs::read_to_string(&profile_path).map_err(|e| PackError::FileSystem {
            operation: "read".to_string(),
            path: profile_path.display().to_string(),
            message: e.to_string(),
        })?;
        let profile = Self::parse_raw(&raw).map_err(|mut e| {
            if let PackError::JsonParse { path, .. } = &mut e {
                *path = profile_path.display().to_string();
            }
            e
        })?;

        // 先递归父版本（父字段为底，子字段覆盖）。
        // MC 版本取值顺序：minecraft_version > client_version（PCL2/HMCL）> 父链。
        let mut merged_mc = profile
            .minecraft_version
            .clone()
            .or_else(|| profile.client_version.clone());
        let mut hints: Vec<String> = Vec::new();
        if let Some(parent_id) = &profile.inherits_from {
            let parent_dir = versions_root.join(parent_id);
            if parent_dir.is_dir() {
                let (parent_mc, parent_loader, parent_lv) =
                    Self::resolve_inherited(&parent_dir, versions_root, seen)?;
                // 父加载器线索并入（子 profile 通常只列差异库）。
                hints.push(loader_hint(&parent_loader));
                if merged_mc.is_none() {
                    merged_mc = Some(parent_mc);
                }
                if let Some(_lv) = parent_lv {
                    // 父加载器版本仅作线索，最终版本以子 profile 关键词提取为准。
                }
            }
        }

        // 收集本目录加载器线索：库名、主类、整份 json 小写文本。
        for lib in &profile.libraries {
            hints.push(lib.name.clone());
        }
        if let Some(main_class) = &profile.main_class {
            hints.push(main_class.clone());
        }
        hints.push(raw.to_lowercase());

        let loader = LoaderKind::from_profile_hints(
            &hints.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        );
        let loader_version = extract_loader_version(&hints, &loader);

        Ok((merged_mc.unwrap_or_else(|| "unknown".to_string()), loader, loader_version))
    }

    /**
     * 函数职责：定位版本目录内的 profile json。
     * 输入说明：dir 为版本目录。
     * 输出说明：命中返回路径；无 json 返回 None。
     * 实现思路：优先 <目录名>.json，其次目录内第一个 .json 文件。
     */
    fn locate_profile(dir: &Path) -> Option<std::path::PathBuf> {
        let dir_name = dir.file_name()?.to_string_lossy().to_string();
        let preferred = dir.join(format!("{dir_name}.json"));
        if preferred.exists() {
            return Some(preferred);
        }
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
    }
}

/**
 * 函数职责：将加载器枚举还原为可并入线索集的关键词。
 * 输入说明：loader 为父链解析出的加载器。
 * 输出说明：对应关键词字符串。
 * 实现思路：一一映射。
 */
fn loader_hint(loader: &LoaderKind) -> String {
    match loader {
        LoaderKind::Vanilla => String::new(),
        LoaderKind::Fabric => "fabric".to_string(),
        LoaderKind::Forge => "forge".to_string(),
        LoaderKind::NeoForge => "neoforge".to_string(),
        LoaderKind::Quilt => "quilt".to_string(),
    }
}

/**
 * 函数职责：从线索集中提取加载器版本号。
 * 输入说明：hints 为全部线索文本；loader 为已判定家族。
 * 输出说明：形如 "0.16.9"、"47.3.0" 或 "21.1.243" 的版本串；未命中返回 None。
 * 实现思路：按家族匹配特征前缀后截取：fabric-loader:x.y / forge / neoforge；
 *           NeoForge 额外兼容启动器参数 "--fml.neoForgeVersion <版本>" 形态。
 */
fn extract_loader_version(hints: &[String], loader: &LoaderKind) -> Option<String> {
    // 常见格式：net.fabricmc:fabric-loader:0.16.9 或 fabricloader:0.16.9。
    let needle = match loader {
        LoaderKind::Fabric => "fabric-loader:",
        LoaderKind::Quilt => "quilt-loader:",
        LoaderKind::Forge => "forge:",
        LoaderKind::NeoForge => "neoforge:",
        LoaderKind::Vanilla => return None,
    };
    for hint in hints {
        if let Some(pos) = hint.find(needle) {
            let rest = &hint[pos + needle.len()..];
            // 截取到下一个非版本字符（版本串为数字与点）。
            let version: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if !version.is_empty() {
                return Some(version);
            }
        }
    }
    // NeoForge 回退：启动器参数形态 "--fmL.neoForgeVersion" 后跟独立版本串。
    if matches!(loader, LoaderKind::NeoForge) {
        for hint in hints {
            if let Some(pos) = hint.find("--fml.neoforgeversion") {
                let rest = &hint[pos + "--fml.neoforgeversion".len()..];
                let version: String = rest
                    .chars()
                    .skip_while(|c| !c.is_ascii_digit())
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                if !version.is_empty() {
                    return Some(version);
                }
            }
        }
    }
    None
}

// ==================== 接口和抽象契约 ====================

/**
 * 结构职责：VersionProfileReader 的默认文件系统实现。
 * 字段说明：无状态。
 * 约束条件：目录不存在时返回 PathUnavailable。
 */
pub struct JsonProfileReader;

impl VersionProfileReader for JsonProfileReader {
    fn read(&self, version_dir: &Path, dir_name: &str) -> PackResult<InstanceProfile> {
        // 目录必须存在且为目录。
        if !version_dir.is_dir() {
            return Err(PackError::PathUnavailable(version_dir.display().to_string()));
        }
        let versions_root = version_dir.parent().unwrap_or(version_dir).to_path_buf();
        let mut seen = Vec::new();
        let (mc_version, loader, loader_version) =
            Self::resolve_inherited(version_dir, &versions_root, &mut seen)?;

        // 关联 jar：优先 <目录名>.jar。
        let jar_name = if version_dir.join(format!("{dir_name}.jar")).exists() {
            dir_name.to_string()
        } else {
            String::new()
        };

        Ok(InstanceProfile {
            version: MinecraftVersion {
                dir_name: dir_name.to_string(),
                jar_name,
            },
            root_dir: version_dir.to_path_buf(),
            profile_path: Self::locate_profile(version_dir),
            mc_version,
            loader,
            loader_version,
            // 占用状态由 ensure_unlocked 按需检测，扫描阶段不做进程枚举（开销大）。
            locked: false,
            locked_by: None,
        })
    }
}
