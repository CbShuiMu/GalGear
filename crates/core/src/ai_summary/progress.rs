//! 分析阶段快照；只传递白名单状态，不向界面暴露对话、工具参数或本地路径。
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 分析任务实际开始或完成的工作阶段。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Checking,
    Resources,
    Sampling,
    Extracting,
    Scripts,
    Save,
    Route,
    Recap,
    Publishing,
}

/// 阶段完成不代表整个任务成功；最终结果仍由调用方的 Result 决定。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Running,
    Completed,
}

/// 某个阶段最近一次回报的状态。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StageProgress {
    pub stage: Stage,
    pub status: StageStatus,
}

/// 单次任务的完整快照；递增序号用于丢弃重复或乱序事件。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Progress {
    pub sequence: u32,
    pub stages: Vec<StageProgress>,
    #[serde(default)]
    pub current_stage: Option<Stage>,
    /// 仅媒体批量脚本提供实际完成批次与文件数；模型规划不能填写此计数。
    #[serde(default)]
    pub batch: Option<BatchProgress>,
}

/// 后台脚本的真实进度，total 为已通过样本验证的提取批次数。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BatchProgress {
    pub completed: u64,
    pub total: u64,
    pub files: u64,
}

pub(super) struct Reporter<'a> {
    snapshot: Progress,
    host_sequence: u32,
    notify: &'a dyn Fn(&Progress),
}

impl<'a> Reporter<'a> {
    pub(super) fn new(notify: &'a dyn Fn(&Progress)) -> Self {
        Self {
            snapshot: Progress::default(),
            host_sequence: 0,
            notify,
        }
    }

    fn update(&mut self, stage: Stage, status: StageStatus) {
        if let Some(item) = self
            .snapshot
            .stages
            .iter_mut()
            .find(|item| item.stage == stage)
        {
            item.status = status;
        } else {
            self.snapshot.stages.push(StageProgress { stage, status });
        }
        self.snapshot.current_stage = if status == StageStatus::Running {
            Some(stage)
        } else {
            self.snapshot
                .stages
                .iter()
                .rfind(|item| item.status == StageStatus::Running)
                .map(|item| item.stage)
        };
    }

    fn send(&mut self) {
        self.snapshot.sequence += 1;
        (self.notify)(&self.snapshot);
    }

    pub(super) fn stage(&mut self, stage: Stage, status: StageStatus) {
        self.update(stage, status);
        self.send();
    }

    pub(super) fn poll(&mut self, work: &Path) {
        // 旧宿主没有此文件；通知损坏或原子替换期间的短暂读取失败不影响业务结果。
        let Some(value) = super::read(&work.join("progress.json"), 16 * 1024)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Progress>(&bytes).ok())
        else {
            return;
        };
        if value.sequence <= self.host_sequence
            || value.stages.len() > 5
            || value
                .stages
                .iter()
                .any(|item| matches!(item.stage, Stage::Checking | Stage::Publishing))
        {
            return;
        }
        self.host_sequence = value.sequence;
        self.snapshot.batch = value
            .batch
            .filter(|batch| batch.total > 0 && batch.completed <= batch.total);
        for item in value.stages {
            self.update(item.stage, item.status);
        }
        if let Some(current) = value.current_stage
            && self
                .snapshot
                .stages
                .iter()
                .any(|item| item.stage == current && item.status == StageStatus::Running)
        {
            self.snapshot.current_stage = Some(current);
        }
        self.send();
    }
}

#[cfg(test)]
mod tests {
    use super::{Progress, Reporter, Stage, StageStatus};
    use std::cell::RefCell;

    #[test]
    fn snapshots_keep_completed_stages_and_ignore_duplicate_or_malformed_reports() {
        let directory = tempfile::tempdir().unwrap();
        let events = RefCell::new(Vec::<Progress>::new());
        let notify = |value: &Progress| events.borrow_mut().push(value.clone());
        let mut reporter = Reporter::new(&notify);
        reporter.stage(Stage::Checking, StageStatus::Completed);
        let path = directory.path().join("progress.json");
        std::fs::write(&path, r#"{"sequence":3,"stages":[{"stage":"resources","status":"completed"},{"stage":"scripts","status":"running"}]}"#).unwrap();
        reporter.poll(directory.path());
        reporter.poll(directory.path());
        std::fs::write(
            &path,
            r#"{"sequence":4,"stages":[{"stage":"secret","status":"completed"}]}"#,
        )
        .unwrap();
        reporter.poll(directory.path());
        assert_eq!(events.borrow().len(), 2);
        assert_eq!(events.borrow()[1].stages.len(), 3);
        assert_eq!(events.borrow()[1].stages[1].status, StageStatus::Completed);
    }
}
