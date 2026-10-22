//! テスト専用: イベント列の全列挙の道具(`docs/tasks/mech-ecrs-2026-10-21/core-exhaustive-guide.md`)。
//!
//! 「小さな有限の語彙から長さ N までのすべての列を状態機械へ流し、各状態で性質を検査する」ための深さ優先の探索だけを
//! 持つ。状態機械(`ImeModel::reduce` など)・語彙・検査は呼び出し側が決める。重複を避けるため、列挙の骨組みはここに
//! 1 つだけ置く(決定表の全列挙 `explicit_press_exhaustive` は状態×キーの直積で、列は扱わない別の方式)。

/// `init` から始め、`vocab` の要素を最大 `max_len` 個まで並べたすべての列を `step` で辿る。
///
/// - `step(state, event)` が `None` を返した枝は、その列を現実に起きえないものとして切る(前提の表明)。
/// - `visit(state, trail)` は、長さ 0(初期状態)を含む**到達したすべての節**で呼ぶ。`trail` はそこまでのイベント列。
/// - 戻り値は訪れた節の数(長さ 0 を含む)。
pub(crate) fn explore<S: Clone, E: Copy>(
    init: &S,
    vocab: &[E],
    max_len: usize,
    step: &dyn Fn(&S, E) -> Option<S>,
    visit: &mut dyn FnMut(&S, &[E]),
) -> u64 {
    fn go<S: Clone, E: Copy>(
        state: &S,
        trail: &mut Vec<E>,
        vocab: &[E],
        max_len: usize,
        step: &dyn Fn(&S, E) -> Option<S>,
        visit: &mut dyn FnMut(&S, &[E]),
    ) -> u64 {
        visit(state, trail);
        let mut nodes = 1;
        if trail.len() == max_len {
            return nodes;
        }
        for &e in vocab {
            if let Some(next) = step(state, e) {
                trail.push(e);
                nodes += go(&next, trail, vocab, max_len, step, visit);
                trail.pop();
            }
        }
        nodes
    }
    go(init, &mut Vec::new(), vocab, max_len, step, visit)
}

#[cfg(test)]
mod tests {
    use super::explore;

    #[test]
    fn counts_all_sequences_up_to_max_len() {
        // 語彙 2 つ・長さ 3 まで: 1 + 2 + 4 + 8 = 15 節。
        let n = explore(&(), &[0u8, 1u8], 3, &|_, _| Some(()), &mut |_, _| {});
        assert_eq!(n, 15);
    }

    #[test]
    fn step_returning_none_prunes_the_branch() {
        // 状態 = 直前の要素。同じ要素の連続を禁じる: 長さ 2 まで 1 + 2 + 2 = 5 節。
        let n = explore(
            &None::<u8>,
            &[0u8, 1u8],
            2,
            &|prev, e| {
                if *prev == Some(e) {
                    None
                } else {
                    Some(Some(e))
                }
            },
            &mut |_, _| {},
        );
        assert_eq!(n, 5);
    }
}
