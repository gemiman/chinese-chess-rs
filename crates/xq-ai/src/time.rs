//! 停止信号与中断控制。
//!
//! # 零 IO 约束下的时间处理
//!
//! [ADR-005](../../../docs/14-决策记录ADR.md#adr-005) 禁止 `xq-ai` 读时钟。
//! 因此时间控制**由外部注入**：本模块只定义「什么时候该停」的抽象，
//! 具体实现（读系统时钟、用户点「停止」、节点上限）由应用层提供。
//!
//! 这个设计的副产品是**测试友好**：用 [`NodeLimitedStop`] 时搜索结果完全可复现，
//! 不受机器性能影响 —— 这正是棋力回归测试需要的性质。

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// 停止信号源。
pub trait StopSignal: Send + Sync {
    /// 是否应当立即停止搜索。
    fn should_stop(&self) -> bool;
}

/// 永不停止。用于「搜完给定深度为止」的场景与测试。
#[derive(Debug, Default, Clone, Copy)]
pub struct NeverStop;

impl StopSignal for NeverStop {
    #[inline]
    fn should_stop(&self) -> bool {
        false
    }
}

/// 外部置位的停止开关。客户端点「停止」、或超时回调使用。
#[derive(Debug, Default)]
pub struct AtomicStop {
    flag: AtomicBool,
}

impl AtomicStop {
    /// 新建（未触发状态）。
    pub const fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
        }
    }

    /// 置位，请求停止。
    pub fn request_stop(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// 清除标志，复用同一个实例。
    pub fn reset(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }
}

impl StopSignal for AtomicStop {
    #[inline]
    fn should_stop(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

/// 节点数上限。
///
/// **这是做可复现棋力测试的唯一正确方式**：同一个种子 + 同一个节点上限，
/// 在任何机器上都会得到完全相同的搜索结果。而按时间限制会导致结果随机器性能漂移，
/// 让「引擎变强了还是变弱了」无法判断。
#[derive(Debug)]
pub struct NodeLimitedStop {
    limit: u64,
    seen: AtomicU64,
}

impl NodeLimitedStop {
    /// 新建，`limit` 为允许搜索的最大节点数。
    pub const fn new(limit: u64) -> Self {
        Self {
            limit,
            seen: AtomicU64::new(0),
        }
    }

    /// 报告已搜索的节点数（由搜索循环调用）。
    #[inline]
    pub fn report(&self, nodes: u64) {
        self.seen.store(nodes, Ordering::Relaxed);
    }

    /// 已用节点数。
    #[inline]
    pub fn seen(&self) -> u64 {
        self.seen.load(Ordering::Relaxed)
    }
}

impl StopSignal for NodeLimitedStop {
    #[inline]
    fn should_stop(&self) -> bool {
        self.seen.load(Ordering::Relaxed) >= self.limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_stop_never_stops() {
        assert!(!NeverStop.should_stop());
    }

    #[test]
    fn atomic_stop_toggles() {
        let stop = AtomicStop::new();
        assert!(!stop.should_stop());
        stop.request_stop();
        assert!(stop.should_stop());
        stop.reset();
        assert!(!stop.should_stop());
    }

    #[test]
    fn node_limited_stop_triggers_at_limit() {
        let stop = NodeLimitedStop::new(100);
        assert!(!stop.should_stop());
        stop.report(99);
        assert!(!stop.should_stop());
        stop.report(100);
        assert!(stop.should_stop());
    }
}
