//! タスクトレイとウィンドウのアイコンの画像(SPEC.md §4.5.2)。
//!
//! 画像ファイルを同梱せず、コードで描く。青い角丸の板に白いキーを 2 段と、スペースキーを並べた
//! キーボードの図案。小さく表示しても形が分かるよう、細部は描かない。

/// 板の色(RGB)。
const BODY: [u8; 3] = [0x2B, 0x57, 0x97];

/// キーの色(RGB)。
const KEY: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// 1 ピクセルを何分割して塗るか(縁を滑らかにする)。
const SUPERSAMPLE: u32 = 4;

/// 図案の部品(0〜1 の座標の矩形と角の半径、色)。
struct Part {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    radius: f32,
    color: [u8; 3],
}

impl Part {
    fn contains(&self, x: f32, y: f32) -> bool {
        if x < self.x0 || x > self.x1 || y < self.y0 || y > self.y1 {
            return false;
        }
        // 角の丸め: 角の円の外側は除く
        let cx = x.clamp(self.x0 + self.radius, self.x1 - self.radius);
        let cy = y.clamp(self.y0 + self.radius, self.y1 - self.radius);
        let (dx, dy) = (x - cx, y - cy);
        dx * dx + dy * dy <= self.radius * self.radius
    }
}

/// 後に並べたものが上に重なる。
fn parts() -> Vec<Part> {
    let mut parts = vec![Part {
        x0: 0.02,
        y0: 0.18,
        x1: 0.98,
        y1: 0.82,
        radius: 0.10,
        color: BODY,
    }];
    // キー: 4 つずつ 2 段
    let (left, right, gap) = (0.12, 0.88, 0.06);
    let width = (right - left - gap * 3.0) / 4.0;
    for (y0, y1) in [(0.28, 0.41), (0.46, 0.59)] {
        for i in 0..4 {
            let x0 = left + i as f32 * (width + gap);
            parts.push(Part {
                x0,
                y0,
                x1: x0 + width,
                y1,
                radius: 0.02,
                color: KEY,
            });
        }
    }
    // スペースキー
    parts.push(Part {
        x0: 0.27,
        y0: 0.64,
        x1: 0.73,
        y1: 0.73,
        radius: 0.02,
        color: KEY,
    });
    parts
}

/// `size` × `size` の RGBA(1 ピクセル 4 バイト、行は上から)。背景は透明。
pub fn keyboard_rgba(size: u32) -> Vec<u8> {
    let parts = parts();
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let n = SUPERSAMPLE;
    for py in 0..size {
        for px in 0..size {
            let mut sum = [0u32; 3];
            let mut covered = 0u32;
            for sy in 0..n {
                for sx in 0..n {
                    let x = (px as f32 + (sx as f32 + 0.5) / n as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / n as f32) / size as f32;
                    if let Some(part) = parts.iter().rev().find(|p| p.contains(x, y)) {
                        for (s, c) in sum.iter_mut().zip(part.color) {
                            *s += u32::from(c);
                        }
                        covered += 1;
                    }
                }
            }
            // どの部品にもかからなければ透明(すべて 0)
            for s in sum {
                out.push(s.checked_div(covered).unwrap_or(0) as u8);
            }
            out.push((covered * 255 / (n * n)) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn draws_a_keyboard_on_transparent_background() {
        for size in [16, 20, 24, 32, 48] {
            let rgba = keyboard_rgba(size);
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            // 上下の端は透明
            assert_eq!(pixel(&rgba, size, size / 2, 0)[3], 0);
            assert_eq!(pixel(&rgba, size, size / 2, size - 1)[3], 0);
            // 板の左端寄りは不透明の板の色
            let body = pixel(&rgba, size, size / 16, size / 2);
            assert_eq!(body, [BODY[0], BODY[1], BODY[2], 255], "{size}");
        }
        // 大きければキーが白く見える
        let rgba = keyboard_rgba(48);
        let key = pixel(&rgba, 48, 10, 16);
        assert_eq!(key, [255, 255, 255, 255]);
    }
}
