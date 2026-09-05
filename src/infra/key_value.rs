//! 文件职责：实现 OptionsParser 与 MergePolicy：key:value 行解析与白名单合并策略。
//! 定义范围：KeyValueParser、WhitelistPolicy 与值合法性校验辅助函数。

use crate::domain::instance::{OptionsParser, ParsedOptions};
use crate::domain::merge::{MergeAction, MergePolicy};
// 白名单常量在 is_whitelisted / value_is_plausible / classify 中消费。
use crate::services::options_merge::{KEY_BINDING_PREFIX, PREFERENCE_KEYS, SOUND_PREFIX, VIEW_PREFIXES};

// ==================== 接口和抽象契约 ====================

/**
 * 结构职责：options.txt 的 key:value 行格式解析器。
 * 字段说明：无状态。
 * 约束条件：容忍空行/注释；值保留原始引号；BOM 剥离；UTF-8 无效字节以替换符保留。
 */
pub struct KeyValueParser;

impl OptionsParser for KeyValueParser {
    fn parse(&self, raw: &str) -> ParsedOptions {
        let mut result = ParsedOptions::default();
        // 剥离 UTF-8 BOM，避免首个键名携带不可见前缀。
        let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
        for line in raw.lines() {
            // 首个 ':' 分隔键值；values 内可能含 ':'（如 key_ 冒号段），只能按首个切分。
            match line.split_once(':') {
                Some((key, value)) => {
                    // 重复键后值覆盖前值，与游戏加载行为一致。
                    result.entries.insert(key.to_string(), value.to_string());
                }
                None => {
                    // 空行与注释行不丢行，进 skipped_lines 供诊断。
                    result.skipped_lines.push(line.to_string());
                }
            }
        }
        result
    }
}

/**
 * 结构职责：内置白名单合并策略：键位优先、音量/视角/语言放行、其余智能忽略。
 * 字段说明：无状态；判定所需的键族常量来自 services::options_merge。
 * 约束条件：必须为纯函数；对 key_ 前缀键在 new_exists=false 时同样放行（补清新键位）。
 */
pub struct WhitelistPolicy;

impl MergePolicy for WhitelistPolicy {
    fn classify(&self, key: &str, new_exists: bool) -> MergeAction {
        // 键位绑定族：旧值优先；目标未列出的旧键位由引擎保留并标记未验证。
        if key.starts_with(KEY_BINDING_PREFIX) {
            return MergeAction::TakeOldBinding;
        }
        // 同名键不在新版中：视为旧版遗留键，智能忽略，不写入新版。
        if !new_exists {
            return MergeAction::DropLegacy;
        }
        // 白名单族（音量/视角/语言/画质）：同名键采用旧值。
        if Self::is_whitelisted(key) {
            return MergeAction::TakeOld;
        }
        // 其余键一律保留新版默认值，杜绝全量覆盖。
        MergeAction::KeepNew
    }
}

// ==================== 函数和方法定义 ====================

impl WhitelistPolicy {
    /**
     * 函数职责：判定键是否属于音量/视角/语言等允许迁移的白名单族。
     * 输入说明：key 为 options 键名。
     * 输出说明：命中白名单返回 true。
     * 实现思路：前缀匹配 SOUND_PREFIX / VIEW_PREFIXES，全等匹配 PREFERENCE_KEYS。
     */
    pub fn is_whitelisted(key: &str) -> bool {
        if key.starts_with(SOUND_PREFIX) {
            return true;
        }
        if VIEW_PREFIXES.iter().any(|prefix| key.starts_with(prefix)) {
            return true;
        }
        PREFERENCE_KEYS.contains(&key)
    }

    /**
     * 函数职责：校验旧值在新版语境下是否合法（防止脏值破坏新版配置）。
     * 输入说明：key 与旧值原始文本。
     * 输出说明：合法返回 true；不合法时调用方应回退 KeepNew。
     * 实现思路：数值类键尝试解析为数值；布尔类键限定 true/false；其余放行。
     */
    pub fn value_is_plausible(key: &str, value: &str) -> bool {
        // 布尔类键：严格限定 true/false。
        if value == "true" || value == "false" {
            return true;
        }
        // 数值类键：必须可解析为 f64（含负数与小数）。
        let numeric_keys = [
            "fov", "gamma", "guiScale", "renderDistance", "maxFps", "simulationDistance",
            "entityDistanceScaling", "fovEffectScale", "screenEffectScale", "mipmapLevels",
        ];
        if numeric_keys.contains(&key) {
            return value.parse::<f64>().is_ok();
        }
        // 其余键不做值校验，交由白名单策略控制。
        true
    }

}
