//! キーマップ指紋の計算(ADR-195段階8、ADR-196決定1e・T5後続の実配線)。
//!
//! 指紋は「学習表が測った構成」を識別する不透明な値で、**キーマップ設定の生の入力**
//! (GJI: `session_keymap`/`custom_keymap_table`/`overlay_keymaps`、Microsoft IME本体:
//! キー割り当て3 DWORD)から作る。`KeyEffectKeymap`の真偽値(overlayの有無・再割り当ての
//! 有無)から作ると、overlayの中身だけの変更や「0以外→別の0以外」の再割り当て変更を
//! 見逃すため使わない。
//!
//! IME種別(GJI / Microsoft IME本体)を種別タグとして混ぜるので、同じ生の値でも別IMEなら
//! 別の指紋になる(`preset`に相当する情報は`session_keymap`の値に含まれる)。
//!
//! ハッシュはFNV-1a(64bit)を2レーン(別の初期値)で走らせた安定な実装で、`std`の
//! `DefaultHasher`(版を越えた値の保証が無い)は使わない。永続化された値と将来のビルドの
//! 値を比較するため、この実装を変えると既存表の指紋が全て不一致になる点に注意
//! (変えるならスキーマ版を上げるか、意図的な全失効として扱うこと)。

use crate::persist::Fingerprint;

const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const FNV_OFFSET_A: u64 = 0xcbf2_9ce4_8422_2325;
/// 第2レーンの初期値(第1レーンと独立にするための別定数。値自体に意味は無い)。
const FNV_OFFSET_B: u64 = 0x8422_2325_cbf2_9ce4;

const KIND_GJI: u8 = 1;
const KIND_MSIME_NATIVE: u8 = 2;

struct Hasher2 {
    a: u64,
    b: u64,
}

impl Hasher2 {
    fn new(kind: u8) -> Self {
        let mut h = Self {
            a: FNV_OFFSET_A,
            b: FNV_OFFSET_B,
        };
        h.byte(kind);
        h
    }

    fn byte(&mut self, x: u8) {
        self.a = (self.a ^ u64::from(x)).wrapping_mul(FNV_PRIME);
        // 第2レーンは入力を反転して混ぜ、第1レーンと衝突パターンを揃えない。
        self.b = (self.b ^ u64::from(!x)).wrapping_mul(FNV_PRIME);
    }

    fn u64(&mut self, x: u64) {
        for b in x.to_le_bytes() {
            self.byte(b);
        }
    }

    fn opt_i64(&mut self, x: Option<i64>) {
        match x {
            None => self.byte(0),
            Some(v) => {
                self.byte(1);
                self.u64(v.cast_unsigned());
            }
        }
    }

    fn opt_u32(&mut self, x: Option<u32>) {
        match x {
            None => self.byte(0),
            Some(v) => {
                self.byte(1);
                self.u64(u64::from(v));
            }
        }
    }

    const fn finish(&self) -> Fingerprint {
        Fingerprint(self.a, self.b)
    }
}

/// GJI(`config1.db`)のキーマップ指紋。`overlay_keymaps`は値そのもの(順序込み)を混ぜる。
#[must_use]
pub fn gji_keymap_fingerprint(
    session_keymap: Option<i64>,
    custom_keymap_table: Option<&str>,
    overlay_keymaps: &[i64],
) -> Fingerprint {
    let mut h = Hasher2::new(KIND_GJI);
    h.opt_i64(session_keymap);
    match custom_keymap_table {
        None => h.byte(0),
        Some(s) => {
            h.byte(1);
            h.u64(s.len() as u64);
            for b in s.bytes() {
                h.byte(b);
            }
        }
    }
    h.u64(overlay_keymaps.len() as u64);
    for &o in overlay_keymaps {
        h.u64(o.cast_unsigned());
    }
    h.finish()
}

/// Microsoft IME本体のキー割り当ての指紋。`assignment_enabled`は
/// `IsKeyAssignmentEnabled == 1`(それ以外の値・不在は全て「無効」で挙動が同じなので真偽値)、
/// `henkan`/`muhenkan`は`KeyAssignmentHenkan`/`KeyAssignmentMuhenkan`の生のDWORD値
/// (マスタースイッチが無効でも値は混ぜる。「0以外→別の0以外」の変更を識別するため)。
#[must_use]
pub fn msime_native_keymap_fingerprint(
    assignment_enabled: bool,
    henkan: Option<u32>,
    muhenkan: Option<u32>,
) -> Fingerprint {
    let mut h = Hasher2::new(KIND_MSIME_NATIVE);
    h.byte(u8::from(assignment_enabled));
    h.opt_u32(henkan);
    h.opt_u32(muhenkan);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_input_gives_same_fingerprint() {
        assert_eq!(
            gji_keymap_fingerprint(Some(1), Some("t"), &[5, 6]),
            gji_keymap_fingerprint(Some(1), Some("t"), &[5, 6])
        );
        assert_eq!(
            msime_native_keymap_fingerprint(true, Some(1), None),
            msime_native_keymap_fingerprint(true, Some(1), None)
        );
    }

    #[test]
    fn gji_session_keymap_change_changes_fingerprint() {
        let base = gji_keymap_fingerprint(Some(1), None, &[]);
        assert_ne!(base, gji_keymap_fingerprint(Some(2), None, &[]));
        assert_ne!(base, gji_keymap_fingerprint(None, None, &[]));
    }

    #[test]
    fn gji_custom_table_change_changes_fingerprint() {
        assert_ne!(
            gji_keymap_fingerprint(Some(0), Some("a"), &[]),
            gji_keymap_fingerprint(Some(0), Some("b"), &[])
        );
        // 不在と空文字列も区別する。
        assert_ne!(
            gji_keymap_fingerprint(Some(0), None, &[]),
            gji_keymap_fingerprint(Some(0), Some(""), &[])
        );
    }

    /// (d) overlayの中身だけが違う(どちらも「overlayあり」)指紋は区別できる。
    #[test]
    fn gji_overlay_content_change_changes_fingerprint() {
        assert_ne!(
            gji_keymap_fingerprint(Some(1), None, &[100]),
            gji_keymap_fingerprint(Some(1), None, &[101])
        );
        assert_ne!(
            gji_keymap_fingerprint(Some(1), None, &[]),
            gji_keymap_fingerprint(Some(1), None, &[100])
        );
        // 要素の切れ目の曖昧さ(長さ接頭辞)で衝突しない。
        assert_ne!(
            gji_keymap_fingerprint(Some(1), None, &[1, 2]),
            gji_keymap_fingerprint(Some(1), None, &[1])
        );
    }

    /// (e) 「0以外→別の0以外」の再割り当て値の変更を区別できる。
    #[test]
    fn msime_native_reassignment_value_change_changes_fingerprint() {
        assert_ne!(
            msime_native_keymap_fingerprint(true, Some(1), Some(1)),
            msime_native_keymap_fingerprint(true, Some(1), Some(2))
        );
        assert_ne!(
            msime_native_keymap_fingerprint(true, Some(1), None),
            msime_native_keymap_fingerprint(true, Some(1), Some(0))
        );
        assert_ne!(
            msime_native_keymap_fingerprint(true, None, None),
            msime_native_keymap_fingerprint(false, None, None)
        );
    }

    /// 同じ生の値でもIME種別が違えば別の指紋(GJI表をMS-IME本体の下で読んで棄却できる)。
    #[test]
    fn different_ime_kinds_never_collide_on_default_configs() {
        assert_ne!(
            gji_keymap_fingerprint(None, None, &[]),
            msime_native_keymap_fingerprint(false, None, None)
        );
    }

    /// 永続化された値と比較する安定性の固定。値が変わったら既存表が全て失効する変更なので、
    /// 意図的でなければ実装を戻すこと。
    #[test]
    fn fingerprint_values_are_pinned() {
        assert_eq!(
            gji_keymap_fingerprint(Some(1), None, &[]),
            Fingerprint(2_670_635_959_025_197_826, 18_032_306_859_591_972_538)
        );
    }
}
