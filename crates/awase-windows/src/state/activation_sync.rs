//! Engine activation echo の actuation 可否を決める純粋判定。

/// ActivationSync 発行時点の open belief に、IME が既に追随したと考えられる
/// 根拠があるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivationSyncBelief {
    /// 明示意図・打鍵予測・実観測がなく、安全デフォルトしかない（または完全に空）。
    Unobserved,
    /// 明示意図・打鍵予測・実観測のいずれかに基づく。
    Grounded,
}

/// ActivationSync の SetOpen を実 IME へ適用する必要があるか。
///
/// 自動 OFF は所有者方針上つねに抑止する。自動 ON も、起動時の未観測状態を
/// active にする一回に相当する場合だけ許可する。
#[must_use]
pub(crate) const fn should_actuate_activation_sync(
    open: bool,
    belief: ActivationSyncBelief,
) -> bool {
    open && matches!(belief, ActivationSyncBelief::Unobserved)
}

#[cfg(test)]
mod tests {
    use super::{should_actuate_activation_sync, ActivationSyncBelief};

    #[test]
    fn actuates_only_open_with_unobserved_belief() {
        assert!(should_actuate_activation_sync(
            true,
            ActivationSyncBelief::Unobserved
        ));
        assert!(!should_actuate_activation_sync(
            false,
            ActivationSyncBelief::Unobserved
        ));
        assert!(!should_actuate_activation_sync(
            true,
            ActivationSyncBelief::Grounded
        ));
        assert!(!should_actuate_activation_sync(
            false,
            ActivationSyncBelief::Grounded
        ));
    }
}
