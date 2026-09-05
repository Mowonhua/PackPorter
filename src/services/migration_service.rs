//! 文件职责：迁移总编排 —— 依据规则生成计划、执行事务、上报进度。
//! 定义范围：MigrationService 结构、plan_migration / execute_plan / to_actions 实现。

use std::path::PathBuf;

use crate::domain::error::{PackError, PackResult};
use crate::domain::instance::{
    AssetLevel, AssetPlanEntry, AssetRule, DecisionAction, InstanceProfile, MergeDecision,
    MigrationOptions, MigrationPlan, MigrationProgress, TransactionOutcome,
};
use crate::domain::rules::{built_in_rules, RuleRegistry};
use crate::domain::transaction::{MigrationTransaction, TransactionAction};
use crate::services::backup_engine::BackupEngine;
use crate::services::instance_service::InstanceService;
use crate::services::options_merge::OptionsMergeEngine;

// ==================== 数据结构、值对象和 DTO ====================

/**
 * 结构职责：迁移编排器的依赖集合，集中持有全部子服务。
 * 字段说明：事务通过 trait 对象注入以支持测试替换；规则表为内置注册表。
 * 约束条件：编排器不直接触碰文件系统，一切 IO 经由子服务完成。
 */
pub struct MigrationService {
    /// 实例探测服务。
    pub instances: InstanceService,
    /// options 合并引擎。
    pub merge_engine: OptionsMergeEngine,
    /// 备份与事务引擎（绑定目标实例）。
    pub backup: BackupEngine,
    /// 事务执行抽象。
    pub transaction: std::sync::Arc<dyn MigrationTransaction>,
    /// 内置规则注册表。
    pub rules: RuleRegistry,
}

// ==================== 函数和方法定义 ====================

impl MigrationService {
    /**
     * 函数职责：以真实子服务构造编排器。
     * 输入说明：versions_root 为 versions/ 目录。
     * 输出说明：始终成功。
     * 实现思路：组装各子服务，事务实现指向 backup 引擎（默认备份根为空路径，
     *           执行阶段以 plan.backup_dir 为准重新构造）。
     */
    pub fn new(versions_root: PathBuf) -> Self {
        let instances = InstanceService::new(versions_root);
        Self {
            instances,
            merge_engine: OptionsMergeEngine::new(
                crate::services::options_merge::default_parser(),
                crate::services::options_merge::default_policy(),
            ),
            backup: BackupEngine::for_instance(PathBuf::new()),
            transaction: std::sync::Arc::new(BackupEngine::for_instance(PathBuf::new())),
            rules: built_in_rules(),
        }
    }

    /**
     * 函数职责：对比源/目标实例，产出完整迁移计划（不写入任何文件）。
     * 输入说明：source 与 target 为两份实例画像；options 为本次迁移的范围开关。
     * 输出说明：完整计划（携带选项快照）；源与目标同目录时返回 InvalidPlan。
     * 实现思路：遍历规则表并按选项过滤关闭的级别 → 复制型规则按级别扫描源目录
     *           （L2 差集过滤）→ L4 预合并 options → 汇总条目与备份目录命名。
     */
    pub fn plan_migration(
        &self,
        source: &InstanceProfile,
        target: &InstanceProfile,
        options: MigrationOptions,
    ) -> PackResult<MigrationPlan> {
        // 源与目标相同没有迁移意义，直接拒绝。
        if source.root_dir == target.root_dir {
            return Err(PackError::InvalidPlan(
                "源实例与目标实例不能相同".to_string(),
            ));
        }
        let mut entries = Vec::new();
        for rule in &self.rules.entries {
            // 选项关闭的级别整级跳过，不产生条目。
            if !options.allows(rule.level) {
                continue;
            }
            let entry = match rule.level {
                AssetLevel::SmartMerge => self.plan_options_entry(rule, source, target),
                _ => self.plan_copy_entry(rule, source, target),
            };
            entries.push(entry);
        }

        // L4 合并：目标缺失 options.txt 时以"旧值初始化新版"处理；L4 关闭时不合并。
        let options_result = if options.include_options {
            self.merge_engine
                .merge_options(
                    &source.root_dir.join("options.txt"),
                    &target.root_dir.join("options.txt"),
                )
                .ok()
        } else {
            None
        };

        // 备份目录：目标实例 backups/ 下按时间戳命名的子目录集合的根。
        let backup_dir = target.root_dir.join("backups");

        Ok(MigrationPlan {
            options,
            source: source.clone(),
            target: target.clone(),
            entries,
            backup_dir,
            options_result,
        })
    }

    /**
     * 函数职责：执行迁移计划：备份 → 事务执行 → 返回结果。
     * 输入说明：plan 为 plan_migration 产物；confirmed 必须为 true，显式表达用户已确认；
     *           是否备份由 plan.options.auto_backup 决定（计划阶段快照，执行期不回读配置）。
     * 输出说明：事务结果；用户未确认时返回 InvalidPlan；事务失败已自动回滚并返回 RolledBack。
     * 实现思路：校验确认标记 → 以 plan.backup_dir 构造事务引擎 → 可选 backup_before →
     *           to_actions → transaction.execute → 汇总报告（含 L4 摘要）。
     */
    pub fn execute_plan(
        &self,
        plan: &MigrationPlan,
        confirmed: bool,
        progress: &mut dyn FnMut(MigrationProgress),
    ) -> PackResult<TransactionOutcome> {
        // 显式确认门：未确认拒绝执行。
        if !confirmed {
            return Err(PackError::InvalidPlan("迁移未经用户确认".to_string()));
        }
        // 以计划中的目标实例重新绑定事务引擎（备份根随之确定）。
        let engine = BackupEngine::for_instance(plan.target.root_dir.clone());
        // 按计划快照的选项执行增量 Zip 备份（关闭或无可备份文件时跳过/返回空路径）。
        if plan.options.auto_backup {
            let _backup_zip = engine.backup_before(plan, progress)?;
        }
        // 计划展开为逐文件动作清单并进入事务执行。
        let actions = self.to_actions(plan);
        let applied = engine.execute(&actions, progress)?;

        // 汇总面向用户的单行成功报告：只报复制文件数与合并键位数，保持极简。
        let copied = plan
            .entries
            .iter()
            .flat_map(|entry| entry.decisions.iter())
            .filter(|d| d.action == DecisionAction::CopyFromOld)
            .count();
        let merged_keys = plan.options_result.as_ref().map(|r| r.merged.len()).unwrap_or(0);
        Ok(TransactionOutcome {
            success: true,
            rolled_back: false,
            moved_items: applied,
            report: format!("共复制{copied}项，合并键位{merged_keys}项"),
        })
    }

    /**
     * 函数职责：为计划的 L4 合并结果生成将要写回的 options 文本（供 UI 预览）。
     * 输入说明：plan 中承载的 options_result。
     * 输出说明：合并后的 options 文本；计划不含 L4 明细时返回 None。
     * 实现思路：直接序列化 options_result。
     */
    pub fn preview_options(&self, plan: &MigrationPlan) -> Option<String> {
        plan.options_result
            .as_ref()
            .map(|result| self.merge_engine.serialize(result))
    }

    /**
     * 函数职责：将计划转换为扁平的事务动作清单（执行器输入）。
     * 输入说明：plan 为待执行计划。
     * 输出说明：复制动作在前（L1→L2→L3），L4 WriteText 在后，顺序即执行顺序。
     * 实现思路：遍历条目决策生成 CopyFile；options_result 生成对目标 options.txt 的 WriteText。
     */
    pub fn to_actions(&self, plan: &MigrationPlan) -> Vec<TransactionAction> {
        let mut actions = Vec::new();
        for entry in &plan.entries {
            for decision in &entry.decisions {
                if decision.action != DecisionAction::CopyFromOld {
                    continue;
                }
                actions.push(TransactionAction::CopyFile {
                    source: plan.source.root_dir.join(&decision.relative_path),
                    destination: plan.target.root_dir.join(&decision.relative_path),
                });
            }
        }
        // L4：合并文本整体写入目标 options.txt。
        if let Some(result) = &plan.options_result {
            let text = self.merge_engine.serialize(result);
            actions.push(TransactionAction::WriteText {
                destination: plan.target.root_dir.join("options.txt"),
                content: text,
            });
        }
        actions
    }

    /**
     * 函数职责：为单条复制型规则（L1/L2/L3）生成计划条目。
     * 输入说明：rule 为当前规则；source/target 为实例画像。
     * 输出说明：含逐文件决策的条目。
     * 实现思路：源路径不存在返回空条目；目录递归收集文件，逐个判断目标存在性：
     *           L2 同名保留新版（KeepNew），L1/L3 同名覆盖（CopyFromOld）。
     */
    fn plan_copy_entry(
        &self,
        rule: &AssetRule,
        source: &InstanceProfile,
        target: &InstanceProfile,
    ) -> AssetPlanEntry {
        let source_path = source.root_dir.join(&rule.relative_path);
        let mut decisions = Vec::new();
        if !source_path.exists() {
            return AssetPlanEntry { rule: rule.clone(), decisions, total_items: 0 };
        }
        if source_path.is_dir() {
            // 目录：递归收集全部文件，按相对路径逐个决策。
            for file in walkdir::WalkDir::new(&source_path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let relative = file
                    .path()
                    .strip_prefix(&source.root_dir)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                let target_exists = target.root_dir.join(&relative).exists();
                let action = decide_copy_action(rule.level, target_exists);
                decisions.push(MergeDecision { relative_path: relative, action });
            }
        } else {
            // 单文件（如 servers.dat）。
            let relative = rule.relative_path.clone();
            let target_exists = target.root_dir.join(&relative).exists();
            let action = decide_copy_action(rule.level, target_exists);
            decisions.push(MergeDecision { relative_path: relative, action });
        }
        let total_items = decisions.len();
        AssetPlanEntry { rule: rule.clone(), decisions, total_items }
    }

    /**
     * 函数职责：为 L4 规则生成计划条目（决策明细承载于 options_result）。
     * 输入说明：rule 为 L4 规则；source/target 为实例画像。
     * 输出说明：decisions 为空、total_items 为合并键数的条目；源文件缺失时为空条目。
     * 实现思路：源 options.txt 存在时预合并并统计键数。
     */
    fn plan_options_entry(
        &self,
        rule: &AssetRule,
        source: &InstanceProfile,
        target: &InstanceProfile,
    ) -> AssetPlanEntry {
        let old_path = source.root_dir.join("options.txt");
        if !old_path.exists() {
            return AssetPlanEntry { rule: rule.clone(), decisions: Vec::new(), total_items: 0 };
        }
        let total_items = self
            .merge_engine
            .merge_options(&old_path, &target.root_dir.join("options.txt"))
            .map(|r| r.merged.len())
            .unwrap_or(0);
        AssetPlanEntry { rule: rule.clone(), decisions: Vec::new(), total_items }
    }
}

/**
 * 函数职责：依据资产级别与目标存在性决定单文件动作。
 * 输入说明：level 为资产级别；target_exists 为目标是否已有同名文件。
 * 输出说明：CopyFromOld（可复制）或 KeepNew（同名保留新版）。
 * 实现思路：L2 一律同名保留新版；L1/L3 同名覆盖。
 */
fn decide_copy_action(level: AssetLevel, target_exists: bool) -> DecisionAction {
    match level {
        // L2：增量合并，同名一律保留新版。
        AssetLevel::Incremental if target_exists => DecisionAction::KeepNew,
        // L1/L3：直接复制语义，同名覆盖；新实例通常无同名文件。
        _ => DecisionAction::CopyFromOld,
    }
}
