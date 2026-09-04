//! 文件职责：模块 B —— options 类文本的智能合并引擎（白名单 + 规则策略）。
//! 定义范围：OptionsMergeEngine 结构、白名单键族常量与合并/序列化实现。

use std::path::Path;

use crate::domain::error::PackResult;
use crate::domain::instance::{OptionsParser, ParsedOptions};
use crate::domain::merge::{MergeAction, MergeOutcome, MergePolicy, MergeResult};

// ==================== 常量、枚举和类型别名 ====================

/// 键位绑定前缀：此族键旧值优先（L4 核心诉求）。
pub const KEY_BINDING_PREFIX: &str = "key_";

/// 音量类键前缀：允许采用旧值。
pub const SOUND_PREFIX: &str = "soundCategory_";

/// 视角与画质类键前缀：允许采用旧值。
pub const VIEW_PREFIXES: &[&str] = &["fov", "gamma", "guiScale", "renderDistance", "maxFps"];

/// 语言与可访问性等纯客户端偏好键（完整键名）。
/// 语言键为 `language`（Minecraft options.txt 实际键名）；
/// `lang` 为历史遗留写法，保留兼容旧配置但游戏不识别。
pub const PREFERENCE_KEYS: &[&str] =
    &["language", "lang", "fullscreen", "pauseOnLostFocus", "narrator"];

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：合并引擎的运行时依赖集合。
 * 字段说明：parser 决定文本解析格式；policy 决定键的合并倾向。
 * 约束条件：引擎无状态；同一线程可复用同一实例处理多对文件。
 */
pub struct OptionsMergeEngine {
    /// options 文本解析器抽象。
    pub parser: std::sync::Arc<dyn OptionsParser>,
    /// 合并策略抽象。
    pub policy: std::sync::Arc<dyn MergePolicy>,
}

// ==================== 函数和方法定义 ====================

impl OptionsMergeEngine {
    /**
     * 函数职责：以指定解析器与策略构造合并引擎。
     * 输入说明：parser 与 policy 均为抽象对象。
     * 输出说明：始终成功。
     * 实现思路：直接装箱字段。
     */
    pub fn new(
        parser: std::sync::Arc<dyn OptionsParser>,
        policy: std::sync::Arc<dyn MergePolicy>,
    ) -> Self {
        Self { parser, policy }
    }

    /**
     * 函数职责：将旧版 options 按白名单规则合并进新版 options，返回合并结果。
     * 输入说明：old_options_path 为旧实例 options.txt；new_options_path 为新实例 options.txt。
     *           新文件可能尚不存在（全新解压实例），此时以旧值初始化全部白名单键。
     * 输出说明：返回逐键决策与最终键值序列；任一路径不可读时返回 FileSystem 错误。
     *           本方法不写任何文件，写入由事务执行器完成。
     * 实现思路：读两份文本（新版缺失按空文本处理）→ parser.parse → 委托 merge_maps。
     */
    pub fn merge_options(
        &self,
        old_options_path: &Path,
        new_options_path: &Path,
    ) -> PackResult<MergeResult> {
        // 旧版文本必须存在；读取失败转为领域错误并携带路径。
        let old_raw = std::fs::read_to_string(old_options_path).map_err(|e| {
            crate::domain::error::PackError::FileSystem {
                operation: "read".to_string(),
                path: old_options_path.display().to_string(),
                message: e.to_string(),
            }
        })?;
        // 新版可能尚未生成（全新实例），缺失时按空文本合并。
        let new_raw = std::fs::read_to_string(new_options_path).unwrap_or_default();
        let old_map = self.parser.parse(&old_raw);
        let new_map = self.parser.parse(&new_raw);
        Ok(self.merge_maps(&old_map, &new_map))
    }

    /**
     * 函数职责：对内存中的两份键值映射执行纯合并（无文件 IO），供测试与 UI 预览复用。
     * 输入说明：old_map 与 new_map 为已解析键值对。
     * 输出说明：合并结果，语义与 merge_options 完全一致。
     * 实现思路：以新版键序为骨架逐键 classify；键位键先收集新版键族集合以判定淘汰；
     *           旧版独有键区分键位（补清）与遗留键（忽略）；值合法性校验失败回退 KeepNew。
     */
    pub fn merge_maps(&self, old_map: &ParsedOptions, new_map: &ParsedOptions) -> MergeResult {
        let mut outcomes: Vec<MergeOutcome> = Vec::new();
        let mut merged: Vec<(String, String)> = Vec::new();

        // 收集新版键位族集合，供淘汰键位判定。
        let new_binding_keys: Vec<&str> = new_map
            .entries
            .keys()
            .filter(|k| k.starts_with(KEY_BINDING_PREFIX))
            .map(|k| k.as_str())
            .collect();

        // 第一遍：以新版键序为骨架，逐键裁决。
        for (key, new_value) in &new_map.entries {
            let final_value = match old_map.entries.get(key) {
                Some(old_value) => match self.policy.classify(key, true) {
                    MergeAction::TakeOld => {
                        // 值校验失败（脏值）时回退新版默认，防止破坏新版本配置。
                        if crate::infra::key_value::WhitelistPolicy::value_is_plausible(key, old_value)
                        {
                            outcomes.push(MergeOutcome {
                                key: key.clone(),
                                action: MergeAction::TakeOld,
                                old_value: Some(old_value.clone()),
                                new_value: Some(new_value.clone()),
                            });
                            old_value.clone()
                        } else {
                            outcomes.push(MergeOutcome {
                                key: key.clone(),
                                action: MergeAction::KeepNew,
                                old_value: Some(old_value.clone()),
                                new_value: Some(new_value.clone()),
                            });
                            new_value.clone()
                        }
                    }
                    MergeAction::TakeOldBinding => {
                        outcomes.push(MergeOutcome {
                            key: key.clone(),
                            action: MergeAction::TakeOldBinding,
                            old_value: Some(old_value.clone()),
                            new_value: Some(new_value.clone()),
                        });
                        old_value.clone()
                    }
                    // KeepNew / DropLegacy 在同名键场景下都保留新值。
                    _ => {
                        outcomes.push(MergeOutcome {
                            key: key.clone(),
                            action: MergeAction::KeepNew,
                            old_value: Some(old_value.clone()),
                            new_value: Some(new_value.clone()),
                        });
                        new_value.clone()
                    }
                },
                None => {
                    // 新版独有键（新 Mod 新键位）：保留新值。
                    outcomes.push(MergeOutcome {
                        key: key.clone(),
                        action: MergeAction::KeepNew,
                        old_value: None,
                        new_value: Some(new_value.clone()),
                    });
                    new_value.clone()
                }
            };
            merged.push((key.clone(), final_value));
        }

        // 第二遍：处理旧版独有键。键位键补清写入，其余遗留键智能忽略。
        let mut dropped = 0usize;
        for (key, old_value) in &old_map.entries {
            if new_map.entries.contains_key(key) {
                continue;
            }
            if key.starts_with(KEY_BINDING_PREFIX) {
                // 新版键族中不存在的键位视为淘汰；仍存在的旧键位补清进新版。
                if crate::infra::key_value::WhitelistPolicy::is_obsolete_binding(
                    key,
                    &new_binding_keys,
                ) {
                    dropped += 1;
                    outcomes.push(MergeOutcome {
                        key: key.clone(),
                        action: MergeAction::DropLegacy,
                        old_value: Some(old_value.clone()),
                        new_value: None,
                    });
                } else {
                    outcomes.push(MergeOutcome {
                        key: key.clone(),
                        action: MergeAction::TakeOldBinding,
                        old_value: Some(old_value.clone()),
                        new_value: None,
                    });
                    merged.push((key.clone(), old_value.clone()));
                }
            } else if crate::infra::key_value::WhitelistPolicy::is_whitelisted(key) {
                // 白名单偏好键（音量/视角/语言）：新版缺失不代表被淘汰——
                // options.txt 只存非默认值，全新实例常缺失这些键，应采用旧值。
                if crate::infra::key_value::WhitelistPolicy::value_is_plausible(key, old_value) {
                    outcomes.push(MergeOutcome {
                        key: key.clone(),
                        action: MergeAction::TakeOld,
                        old_value: Some(old_value.clone()),
                        new_value: None,
                    });
                    merged.push((key.clone(), old_value.clone()));
                } else {
                    dropped += 1;
                    outcomes.push(MergeOutcome {
                        key: key.clone(),
                        action: MergeAction::DropLegacy,
                        old_value: Some(old_value.clone()),
                        new_value: None,
                    });
                }
            } else {
                // 非键位、非白名单的遗留键一律忽略（含旧 Mod 残留配置）。
                dropped += 1;
                outcomes.push(MergeOutcome {
                    key: key.clone(),
                    action: MergeAction::DropLegacy,
                    old_value: Some(old_value.clone()),
                    new_value: None,
                });
            }
        }

        MergeResult { outcomes, merged, dropped }
    }

    /**
     * 函数职责：将合并结果序列化为 options.txt 文本。
     * 输入说明：result 为 merge_maps 产物。
     * 输出说明：每行 "key:value" 的文本，以换行结尾。
     * 实现思路：逐对拼接键值，冒号分隔。
     */
    pub fn serialize(&self, result: &MergeResult) -> String {
        let mut out = String::new();
        for (key, value) in &result.merged {
            out.push_str(key);
            out.push(':');
            out.push_str(value);
            out.push('\n');
        }
        out
    }
}

/**
 * 函数职责：提供默认 options 解析器（key:value 行格式）。
 * 输入说明：无。
 * 输出说明：infra 层 KeyValueParser 实例。
 * 实现思路：直接构造 infra 类型并装箱。
 */
pub fn default_parser() -> std::sync::Arc<dyn OptionsParser> {
    std::sync::Arc::new(crate::infra::key_value::KeyValueParser)
}

/**
 * 函数职责：提供默认合并策略（键位/音量/视角/语言白名单 + 淘汰键智能忽略）。
 * 输入说明：无。
 * 输出说明：infra 层 WhitelistPolicy 实例。
 * 实现思路：直接构造 infra 类型并装箱。
 */
pub fn default_policy() -> std::sync::Arc<dyn MergePolicy> {
    std::sync::Arc::new(crate::infra::key_value::WhitelistPolicy)
}

/**
 * 函数职责：将合并明细压缩为面向用户的摘要行。
 * 输入说明：outcomes 为逐键决策。
 * 输出说明：形如"键位 12、偏好 4、保留新版 30、忽略 2"的摘要文本。
 * 实现思路：按动作族分组计数后拼接。
 */
pub fn summarize(outcomes: &[MergeOutcome]) -> String {
    let mut bindings = 0usize;
    let mut preferences = 0usize;
    let mut kept_new = 0usize;
    let mut dropped = 0usize;
    for outcome in outcomes {
        match outcome.action {
            MergeAction::TakeOldBinding => bindings += 1,
            MergeAction::TakeOld => preferences += 1,
            MergeAction::KeepNew => kept_new += 1,
            MergeAction::DropLegacy => dropped += 1,
        }
    }
    format!("键位 {bindings}、偏好 {preferences}、保留新版 {kept_new}、忽略 {dropped}")
}
