#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostBypassKey {
    /// latch を維持し、このキー自体は通常処理へ流す。
    KeepArmed,
    /// このキー自体が prefix のコマンドキーだが NICOLA の対象外。
    ConsumesPrefixSilently,
    /// prefix のコマンドキー本体。NICOLA をスキップして素通しし、latch を消費する。
    ConsumeAndPassthrough,
    /// 対応する KeyDown で既に消費済みのはずの孤児 KeyUp。
    PassthroughKeepArmed,
}

/// `vk` そのものではなく分類済み bool を取ることで、`vk.rs` の分類関数を
/// SSOT として保つ。
///
/// 判定順序は意味を持つ。modifier は passthrough の真部分集合なので、
/// modifier 判定を後ろへ動かすと `prefix + Shift + 5` が壊れる。
pub(crate) const fn classify_post_bypass_key(
    is_key_down: bool,
    ctrl_held: bool,
    vk_is_modifier: bool,
    vk_is_passthrough: bool,
) -> PostBypassKey {
    if ctrl_held {
        return PostBypassKey::KeepArmed;
    }
    if vk_is_modifier {
        return PostBypassKey::KeepArmed;
    }
    if vk_is_passthrough {
        return if is_key_down {
            PostBypassKey::ConsumesPrefixSilently
        } else {
            PostBypassKey::KeepArmed
        };
    }
    if is_key_down {
        PostBypassKey::ConsumeAndPassthrough
    } else {
        PostBypassKey::PassthroughKeepArmed
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_post_bypass_key, PostBypassKey};

    #[test]
    fn classification_covers_all_boolean_combinations() {
        let mut seen = [false; 4];
        for is_key_down in [false, true] {
            for ctrl_held in [false, true] {
                for vk_is_modifier in [false, true] {
                    for vk_is_passthrough in [false, true] {
                        let key = classify_post_bypass_key(
                            is_key_down,
                            ctrl_held,
                            vk_is_modifier,
                            vk_is_passthrough,
                        );
                        match key {
                            PostBypassKey::KeepArmed => seen[0] = true,
                            PostBypassKey::ConsumesPrefixSilently => seen[1] = true,
                            PostBypassKey::ConsumeAndPassthrough => seen[2] = true,
                            PostBypassKey::PassthroughKeepArmed => seen[3] = true,
                        }
                    }
                }
            }
        }
        assert_eq!(seen, [true, true, true, true]);
    }

    #[test]
    fn modifier_passthrough_overlap_keeps_latch_for_shift_chord() {
        assert_eq!(
            classify_post_bypass_key(true, false, true, true),
            PostBypassKey::KeepArmed
        );
    }

    #[test]
    fn prefix_arrow_consumes_prefix_silently_and_next_char_is_unarmed() {
        assert_eq!(
            classify_post_bypass_key(true, false, false, true),
            PostBypassKey::ConsumesPrefixSilently
        );
        assert_eq!(
            classify_post_bypass_key(true, false, false, false),
            PostBypassKey::ConsumeAndPassthrough
        );
    }

    #[test]
    fn prefix_shift_then_digit_keeps_then_consumes() {
        assert_eq!(
            classify_post_bypass_key(true, false, true, true),
            PostBypassKey::KeepArmed
        );
        assert_eq!(
            classify_post_bypass_key(true, false, false, false),
            PostBypassKey::ConsumeAndPassthrough
        );
    }
}
