//! キーボードの配列データ。
//!
//! 位置と大きさは「キー 1 個分の幅」(1u)を単位とする。配列は [`Layout`] の値として
//! 差し替えられるので、JIS 配列などは別のコンストラクタを足せば追加できる。

use crate::config::{KeyCode, NUMPAD_ORIGIN_FLAG};

/// 配列上の 1 キー(物理キー)。座標はキーの左上で、単位は 1u。
#[derive(Debug, Clone, PartialEq)]
pub struct KeyDef {
    /// このキーが出しうるキーコード(正規化済み。[`crate::keys::normalize`] を通した値)。
    ///
    /// 先頭が主のコードで、表示名にはこれを使う。テンキーのように NumLock の状態で
    /// 別の VK を出すキーは、残りにそれらを持つ。
    pub codes: Vec<KeyCode>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// キーボードの配列 1 つ分。
#[derive(Debug, Clone)]
pub struct Layout {
    pub keys: Vec<KeyDef>,
}

impl KeyDef {
    /// 主のキーコード。
    pub fn primary(&self) -> KeyCode {
        self.codes[0]
    }
}

impl Layout {
    /// 配列全体の幅と高さ(単位は 1u)。
    pub fn size(&self) -> (f32, f32) {
        self.keys.iter().fold((0.0, 0.0), |(w, h), k| {
            (f32::max(w, k.x + k.w), f32::max(h, k.y + k.h))
        })
    }

    /// US 配列(ANSI 104 キーのフルサイズ)。
    pub fn us_ansi() -> Self {
        let mut b = Builder::default();

        // ファンクション行
        b.row(0.0, 0.0, &[(0x1B, 1.0)]);
        b.row(
            0.0,
            2.0,
            &[(0x70, 1.0), (0x71, 1.0), (0x72, 1.0), (0x73, 1.0)],
        );
        b.row(
            0.0,
            6.5,
            &[(0x74, 1.0), (0x75, 1.0), (0x76, 1.0), (0x77, 1.0)],
        );
        b.row(
            0.0,
            11.0,
            &[(0x78, 1.0), (0x79, 1.0), (0x7A, 1.0), (0x7B, 1.0)],
        );
        b.row(0.0, 15.25, &[(0x2C, 1.0), (0x91, 1.0), (0x13, 1.0)]);

        // メインブロック
        let r1 = 1.5;
        b.row(
            r1,
            0.0,
            &[
                (0xC0, 1.0),
                (0x31, 1.0),
                (0x32, 1.0),
                (0x33, 1.0),
                (0x34, 1.0),
                (0x35, 1.0),
                (0x36, 1.0),
                (0x37, 1.0),
                (0x38, 1.0),
                (0x39, 1.0),
                (0x30, 1.0),
                (0xBD, 1.0),
                (0xBB, 1.0),
                (0x08, 2.0),
            ],
        );
        b.row(
            r1 + 1.0,
            0.0,
            &[
                (0x09, 1.5),
                (0x51, 1.0),
                (0x57, 1.0),
                (0x45, 1.0),
                (0x52, 1.0),
                (0x54, 1.0),
                (0x59, 1.0),
                (0x55, 1.0),
                (0x49, 1.0),
                (0x4F, 1.0),
                (0x50, 1.0),
                (0xDB, 1.0),
                (0xDD, 1.0),
                (0xDC, 1.5),
            ],
        );
        b.row(
            r1 + 2.0,
            0.0,
            &[
                (0x14, 1.75),
                (0x41, 1.0),
                (0x53, 1.0),
                (0x44, 1.0),
                (0x46, 1.0),
                (0x47, 1.0),
                (0x48, 1.0),
                (0x4A, 1.0),
                (0x4B, 1.0),
                (0x4C, 1.0),
                (0xBA, 1.0),
                (0xDE, 1.0),
                (0x0D, 2.25),
            ],
        );
        b.row(
            r1 + 3.0,
            0.0,
            &[
                (0xA0, 2.25),
                (0x5A, 1.0),
                (0x58, 1.0),
                (0x43, 1.0),
                (0x56, 1.0),
                (0x42, 1.0),
                (0x4E, 1.0),
                (0x4D, 1.0),
                (0xBC, 1.0),
                (0xBE, 1.0),
                (0xBF, 1.0),
                (0xA1, 2.75),
            ],
        );
        b.row(
            r1 + 4.0,
            0.0,
            &[
                (0xA2, 1.25),
                (0x5B, 1.25),
                (0xA4, 1.25),
                (0x20, 6.25),
                (0xA5, 1.25),
                (0x5C, 1.25),
                (0x5D, 1.25),
                (0xA3, 1.25),
            ],
        );

        // ナビゲーションクラスタと矢印(テンキー由来フラグなし)
        let nav = 15.25;
        b.row(r1, nav, &[(0x2D, 1.0), (0x24, 1.0), (0x21, 1.0)]);
        b.row(r1 + 1.0, nav, &[(0x2E, 1.0), (0x23, 1.0), (0x22, 1.0)]);
        b.row(r1 + 3.0, nav + 1.0, &[(0x26, 1.0)]);
        b.row(r1 + 4.0, nav, &[(0x25, 1.0), (0x28, 1.0), (0x27, 1.0)]);

        // テンキー。数字と . は NumLock オフ時に出すテンキー由来の VK も持つ。
        // / はエンジンに常にテンキー由来フラグ付きで届く(SPEC.md §3.1)。
        let num = 18.5;
        b.row(r1, num, &[(0x90, 1.0)]);
        b.key(&[np(0x6F)], num + 1.0, r1, 1.0, 1.0);
        b.row(r1, num + 2.0, &[(0x6A, 1.0), (0x6D, 1.0)]);
        b.key(&[0x67, np(0x24)], num, r1 + 1.0, 1.0, 1.0);
        b.key(&[0x68, np(0x26)], num + 1.0, r1 + 1.0, 1.0, 1.0);
        b.key(&[0x69, np(0x21)], num + 2.0, r1 + 1.0, 1.0, 1.0);
        b.key(&[0x6B], num + 3.0, r1 + 1.0, 1.0, 2.0);
        b.key(&[0x64, np(0x25)], num, r1 + 2.0, 1.0, 1.0);
        b.key(&[0x65, 0x0C], num + 1.0, r1 + 2.0, 1.0, 1.0);
        b.key(&[0x66, np(0x27)], num + 2.0, r1 + 2.0, 1.0, 1.0);
        b.key(&[0x61, np(0x23)], num, r1 + 3.0, 1.0, 1.0);
        b.key(&[0x62, np(0x28)], num + 1.0, r1 + 3.0, 1.0, 1.0);
        b.key(&[0x63, np(0x22)], num + 2.0, r1 + 3.0, 1.0, 1.0);
        b.key(&[np(0x0D)], num + 3.0, r1 + 3.0, 1.0, 2.0);
        b.key(&[0x60, np(0x2D)], num, r1 + 4.0, 2.0, 1.0);
        b.key(&[0x6E, np(0x2E)], num + 2.0, r1 + 4.0, 1.0, 1.0);

        Layout { keys: b.keys }
    }
}

/// テンキー由来フラグ付きの値。
const fn np(vk: u32) -> u32 {
    vk | NUMPAD_ORIGIN_FLAG
}

/// 配列データを組み立てるための補助。
#[derive(Default)]
struct Builder {
    keys: Vec<KeyDef>,
}

impl Builder {
    /// キーを 1 つ置く。`codes` は先頭が主のコード。
    fn key(&mut self, codes: &[u32], x: f32, y: f32, w: f32, h: f32) {
        self.keys.push(KeyDef {
            codes: codes.iter().copied().map(KeyCode).collect(),
            x,
            y,
            w,
            h,
        });
    }

    /// `x` から右へ、高さ 1u のキーを隙間なく並べる。`keys` は `(キーコード, 幅)`。
    fn row(&mut self, y: f32, x: f32, keys: &[(u32, f32)]) {
        let mut x = x;
        for &(code, w) in keys {
            self.key(&[code], x, y, w, 1.0);
            x += w;
        }
    }
}

#[cfg(test)]
impl Layout {
    /// `code` を出す物理キーの添字。配列にないコードなら `None`。
    ///
    /// テストでだけ使う(キーボード図は各キーの `codes` から状態を決める)。
    pub fn find(&self, code: KeyCode) -> Option<usize> {
        let code = crate::keys::normalize(code);
        self.keys.iter().position(|k| k.codes.contains(&code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;

    #[test]
    fn us_ansi_has_104_keys() {
        assert_eq!(Layout::us_ansi().keys.len(), 104);
    }

    #[test]
    fn us_ansi_has_no_duplicate_codes_across_keys() {
        let layout = Layout::us_ansi();
        let mut seen = std::collections::HashSet::new();
        for k in &layout.keys {
            for c in &k.codes {
                assert!(seen.insert(*c), "duplicate {:#X}", c.0);
            }
        }
    }

    #[test]
    fn us_ansi_codes_have_names_and_are_normalized() {
        for k in &Layout::us_ansi().keys {
            assert!(!k.codes.is_empty());
            for &c in &k.codes {
                assert!(keys::lookup(c).is_some(), "no name for {:#X}", c.0);
                assert_eq!(keys::normalize(c), c, "{:#X}", c.0);
            }
        }
    }

    #[test]
    fn us_ansi_numpad_keys_have_alternate_codes() {
        let layout = Layout::us_ansi();
        let key = |code: u32| &layout.keys[layout.find(KeyCode(code)).unwrap()];
        assert_eq!(key(0x64).codes, vec![KeyCode(0x64), KeyCode(np(0x25))]);
        assert_eq!(key(0x65).codes, vec![KeyCode(0x65), KeyCode(0x0C)]);
        assert_eq!(key(0x6E).codes, vec![KeyCode(0x6E), KeyCode(np(0x2E))]);
        assert_eq!(key(np(0x0D)).primary(), KeyCode(np(0x0D)));
        // テンキーの / はフラグ付きだけを持つ。フラグなしの 0x6F は配列にない
        assert_eq!(key(np(0x6F)).codes, vec![KeyCode(np(0x6F))]);
        assert_eq!(layout.find(KeyCode(0x6F)), None);
        // フラグなしはナビゲーションクラスタ・メインの Enter
        assert_eq!(key(0x25).codes, vec![KeyCode(0x25)]);
        assert_eq!(key(0x0D).codes, vec![KeyCode(0x0D)]);
    }

    #[test]
    fn us_ansi_keys_do_not_overlap() {
        let keys = Layout::us_ansi().keys;
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                let overlap =
                    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(
                    !overlap,
                    "{:#X} overlaps {:#X}",
                    a.primary().0,
                    b.primary().0
                );
            }
        }
    }

    #[test]
    fn us_ansi_size() {
        assert_eq!(Layout::us_ansi().size(), (22.5, 6.5));
    }
}
