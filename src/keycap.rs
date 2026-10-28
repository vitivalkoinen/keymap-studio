//! キーボード図のキーの上に描く文字の大きさ・配置と、キーの色。
//!
//! 描画(egui の `Painter`)から切り離した計算だけを置き、テストできるようにする。
//! 大きさの単位はすべて px。`unit` は 1u(キー 1 個分の幅)の大きさ。

use eframe::egui::{Color32, Pos2, Rect, pos2};

use crate::matching::KeyState;

/// 1u の大きさの下限と上限(キーボード図はこの範囲で伸縮する)。
pub const MIN_UNIT: f32 = 28.0;
pub const MAX_UNIT: f32 = 52.0;

/// キーの枠を配列上の区画から内側へ縮める幅(隣のキーとの隙間の半分)。
pub const KEY_GAP: f32 = 2.0;

/// キーの枠と文字の間の余白。
pub const PADDING: f32 = 1.0;

/// 2 行の間の隙間。
pub const LINE_GAP: f32 = 1.0;

/// 行の高さとフォントの大きさの比の見積もり。既定のフォントでは実際は 1.15 前後なので、
/// 少し大きめに見積もって重ならないようにする。
pub const ROW_HEIGHT_RATIO: f32 = 1.2;

/// 変換先に対する元の名前の文字の大きさの比。
const NAME_RATIO: f32 = 0.75;

/// 幅が足りないとき、変換先の文字を縮めてよい下限(元の大きさに対する比)。
const MIN_SHRINK: f32 = 0.8;

/// リマップのないキーに名前だけを描くときの文字の大きさ。
pub fn single_size(unit: f32) -> f32 {
    (unit * 0.26).max(9.0)
}

/// リマップ元のキーに描く 2 行(元の名前と変換先)の文字の大きさ `(元の名前, 変換先)`。
///
/// 1u のキーの高さに 2 行が収まる大きさにする。
pub fn two_line_sizes(unit: f32) -> (f32, f32) {
    let available = unit - 2.0 * KEY_GAP - 2.0 * PADDING - LINE_GAP;
    let fit = available / (ROW_HEIGHT_RATIO * (1.0 + NAME_RATIO));
    let target = f32::min(unit * 0.34, fit);
    (target * NAME_RATIO, target)
}

/// 1 行だけ描く文字、または 2 行のうち下の行(変換先)の横幅の上限。
///
/// 右上の印は上の行の高さにあり、下の行とは縦に重ならないので、印の有無に関係しない。
pub fn text_max_width(key_rect: Rect) -> f32 {
    (key_rect.width() - 2.0 * PADDING).max(0.0)
}

/// 2 行のうち上の行(元の名前)の横幅の上限。`marker` が `true` なら、右上の印と
/// 重ならないよう両側を空ける(中央に揃えるので左右対称に空ける)。
pub fn name_max_width(key_rect: Rect, unit: f32, marker: bool) -> f32 {
    if marker {
        let inset = 2.0 * marker_radius(unit) + PADDING;
        (text_max_width(key_rect) - 2.0 * inset).max(0.0)
    } else {
        text_max_width(key_rect)
    }
}

/// 縦に並べた 2 行の中心の y 座標 `(上の行, 下の行)`。2 行をまとめてキーの縦の中央に置く。
/// `top_h` と `bottom_h` は各行の高さ。
pub fn stack_two(key_rect: Rect, top_h: f32, bottom_h: f32) -> (f32, f32) {
    let total = top_h + LINE_GAP + bottom_h;
    let top = key_rect.center().y - total / 2.0;
    (top + top_h / 2.0, top + top_h + LINE_GAP + bottom_h / 2.0)
}

/// 変換先の文字の候補から、描くものと大きさを選ぶ。
///
/// `widths` は各候補を大きさ `size` で並べたときの幅で、先頭ほど望ましい
/// (例: `["→ Disable", "Disable"]`)。縮めて `max_width` に収まる最初の候補を選ぶ。
/// どれも収まらなければ最後の候補を下限の大きさで選ぶ(描くときに末尾を省略する)。
/// 戻り値は `(候補の添字, 大きさ)`。
pub fn choose_fit(widths: &[f32], size: f32, max_width: f32) -> (usize, f32) {
    let min = size * MIN_SHRINK;
    for (i, &w) in widths.iter().enumerate() {
        if w <= max_width {
            return (i, size);
        }
        let scaled = size * max_width / w;
        if scaled >= min {
            return (i, scaled);
        }
    }
    (widths.len().saturating_sub(1), min)
}

/// 「ほかのコードもリマップ元」を示す印の半径。
pub fn marker_radius(unit: f32) -> f32 {
    (unit * 0.06).max(2.0)
}

/// 「ほかのコードもリマップ元」を示す印の中心(キーの右上の角)。
pub fn marker_center(key_rect: Rect, unit: f32) -> Pos2 {
    let r = marker_radius(unit);
    pos2(
        key_rect.right() - PADDING - r - 1.0,
        key_rect.top() + PADDING + r + 1.0,
    )
}

/// キーの状態ごとの塗りの色。リマップのないキーは `None`(テーマの既定の色を使う)。
///
/// 保存済みは橙、未保存は青。色覚の違いがあっても見分けやすい組み合わせにし、
/// 選択中の枠(テーマの最も強い文字色。ほぼ白か黒)とも混ざらない中間の明るさにする。
/// 文字はテーマの強い文字色で描くので、ダークでは暗め、ライトでは明るめの色にする。
pub fn state_fill(state: KeyState, dark_mode: bool) -> Option<Color32> {
    match state {
        KeyState::Plain => None,
        KeyState::Saved => Some(saved_fill(dark_mode)),
        KeyState::Unsaved => Some(unsaved_fill(dark_mode)),
    }
}

/// 保存済みのリマップの色。
pub fn saved_fill(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(0xA0, 0x55, 0x1E)
    } else {
        Color32::from_rgb(0xF5, 0xB0, 0x6B)
    }
}

/// 未保存の変更の色。保存ボタンの強調にも使う。
pub fn unsaved_fill(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(0x23, 0x6A, 0xB8)
    } else {
        Color32::from_rgb(0x8C, 0xC4, 0xF5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    /// 配列上の `w`×`h`(単位は 1u)の区画に置いたキーの枠。
    fn key_rect(unit: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(w * unit, h * unit)).shrink(KEY_GAP)
    }

    fn units() -> impl Iterator<Item = f32> {
        (0..=24).map(|i| MIN_UNIT + i as f32)
    }

    #[test]
    fn two_lines_fit_in_one_unit_key() {
        for unit in units() {
            let rect = key_rect(unit, 1.0, 1.0);
            let (name, target) = two_line_sizes(unit);
            assert!(name < target, "{unit}");
            let (name_h, target_h) = (name * ROW_HEIGHT_RATIO, target * ROW_HEIGHT_RATIO);
            let (top_y, bottom_y) = stack_two(rect, name_h, target_h);
            let top = (top_y - name_h / 2.0, top_y + name_h / 2.0);
            let bottom = (bottom_y - target_h / 2.0, bottom_y + target_h / 2.0);
            // 重ならず、余白を残してキーの中に収まる
            assert!(top.1 <= bottom.0, "{unit}");
            assert!(top.0 >= rect.top() + PADDING - 1e-3, "{unit}");
            assert!(bottom.1 <= rect.bottom() - PADDING + 1e-3, "{unit}");
        }
    }

    #[test]
    fn two_line_sizes_at_smallest_unit_stay_readable() {
        let (name, target) = two_line_sizes(MIN_UNIT);
        assert!(name >= 7.0, "{name}");
        assert!(target >= 9.5, "{target}");
        // 大きいときは伸びるが、上限で頭打ちにする
        let (name, target) = two_line_sizes(MAX_UNIT);
        assert!(name > 10.0 && target <= MAX_UNIT * 0.34 + 1e-3);
    }

    #[test]
    fn stack_two_centers_block_vertically() {
        let rect = key_rect(40.0, 1.0, 2.0);
        let (a, b) = stack_two(rect, 10.0, 14.0);
        let top = a - 5.0;
        let bottom = b + 7.0;
        assert!((rect.center().y - (top + bottom) / 2.0).abs() < 1e-3);
    }

    #[test]
    fn marker_does_not_overlap_text_area() {
        for unit in units() {
            for (w, h) in [(1.0, 1.0), (1.0, 2.0), (2.0, 1.0)] {
                let rect = key_rect(unit, w, h);
                let c = marker_center(rect, unit);
                let r = marker_radius(unit);
                assert!(rect.contains(c + vec2(r, -r)) && rect.contains(c + vec2(r, r)));

                // 上の行: 中央揃えの文字が占めうる右端より、印の左端が右にある
                let name_w = name_max_width(rect, unit, true);
                assert!(name_w > 0.0, "{unit}");
                let name_right = rect.center().x + name_w / 2.0;
                assert!(c.x - r >= name_right - 1e-3, "{unit}");

                // 下の行: 印より下にあるので、横幅は印がないときと同じ
                let (name, target) = two_line_sizes(unit);
                let (_, target_y) =
                    stack_two(rect, name * ROW_HEIGHT_RATIO, target * ROW_HEIGHT_RATIO);
                let target_top = target_y - target * ROW_HEIGHT_RATIO / 2.0;
                assert!(target_top >= c.y + r, "{unit} {w}x{h}");
            }
        }
        let rect = key_rect(MIN_UNIT, 1.0, 1.0);
        assert_eq!(name_max_width(rect, MIN_UNIT, false), text_max_width(rect));
        assert!(name_max_width(rect, MIN_UNIT, true) < text_max_width(rect));
    }

    #[test]
    fn choose_fit_prefers_first_candidate_then_shrinks_then_falls_back() {
        // 収まる
        assert_eq!(choose_fit(&[20.0, 10.0], 10.0, 22.0), (0, 10.0));
        // 少し縮めれば収まる
        let (i, size) = choose_fit(&[25.0, 15.0], 10.0, 22.0);
        assert_eq!(i, 0);
        assert!((size - 8.8).abs() < 1e-3);
        // 縮めても収まらないので矢印のない候補
        assert_eq!(choose_fit(&[40.0, 20.0], 10.0, 22.0), (1, 10.0));
        // どれも収まらなければ最後の候補を下限の大きさで(省略して描く)
        assert_eq!(choose_fit(&[80.0, 60.0], 10.0, 22.0), (1, 8.0));
    }

    #[test]
    fn state_colors_are_distinct_in_both_themes() {
        for dark in [false, true] {
            let saved = state_fill(KeyState::Saved, dark).unwrap();
            let unsaved = state_fill(KeyState::Unsaved, dark).unwrap();
            assert_ne!(saved, unsaved);
            assert_eq!(state_fill(KeyState::Plain, dark), None);
            // 選択の枠(白か黒)と混ざらない中間の明るさ
            for c in [saved, unsaved] {
                let luma = c.r() as u32 * 299 + c.g() as u32 * 587 + c.b() as u32 * 114;
                let luma = luma / 1000;
                assert!((60..=210).contains(&luma), "{c:?} {luma}");
            }
            // 色相が大きく違う(橙は赤が強く、青は青が強い)
            assert!(saved.r() > saved.b() && unsaved.b() > unsaved.r());
        }
    }
}
