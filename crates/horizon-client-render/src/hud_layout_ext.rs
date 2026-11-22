//! Render-only extensions to hud_layout: functions that need `crate::hud`
//! (the HUD batch/atlas) or `crate::font`, which are not in horizon-sim-data.
use crate::font::Font;
use crate::hud_layout::{Blend, GuiDraw, IDENTITY, pose_apply};
use crate::text::Text;

/// `Font.width` of styled text.
pub fn width(font: &Font, t: &Text) -> i32 {
    font.width_text(t) as i32
}

/// An ARGB colour as the HUD batch's packed RGBA.
pub fn abgr(color: u32) -> u32 {
    let [a, r, g, b] = color.to_be_bytes();
    crate::hud::rgba(r, g, b, a)
}

pub fn paint(batch: &mut crate::hud::HudBatch, atlas: &crate::hud::HudAtlas, draws: &[GuiDraw]) {
    let mut stack = vec![IDENTITY];
    // One buffer for the sprite keys, instead of a string per drawn sprite.
    let mut name = String::new();
    for d in draws {
        let start = batch.vertices.len();
        let pose = *stack.last().unwrap_or(&IDENTITY);
        match d {
            GuiDraw::Sprite {
                sprite,
                x,
                y,
                w,
                h,
                region,
                color,
                blend,
            } => {
                // Effect sprites belong to the mob_effect atlas directory (a
                // content mod's under its namespace); the other blitSprite
                // ids refer to GUI sprites.
                name.clear();
                let effect = sprite.starts_with("mob_effect/")
                    || sprite
                        .split_once(':')
                        .is_some_and(|(_, path)| path.starts_with("mob_effect/"));
                if !effect {
                    name.push_str("gui/sprites/");
                }
                name.push_str(sprite);
                let found = match effect {
                    true => atlas.sprite_or_missing(&name),
                    false => atlas.sprites.get(name.as_str()).copied(),
                };
                let Some(mut r) = found else {
                    continue;
                };
                if let Some([sw, sh, u, v]) = *region {
                    let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                    let u0 = r.u0 + du * u as f32 / sw as f32;
                    let v0 = r.v0 + dv * v as f32 / sh as f32;
                    r.u1 = u0 + du * *w as f32 / sw as f32;
                    r.v1 = v0 + dv * *h as f32 / sh as f32;
                    r.u0 = u0;
                    r.v0 = v0;
                }
                let at = [*x as f32, *y as f32, *w as f32, *h as f32];
                match crate::hud::sprite_modifiers::draws(atlas, &name, r, at, abgr(*color)) {
                    // A Polytone screen sprite modifier: the sprite changed, then its overlays.
                    Some(draws) => {
                        for d in draws {
                            batch.rect_uv(d.at[0], d.at[1], d.at[2], d.at[3], d.rect, d.color);
                        }
                    }
                    None => batch.rect_uv(at[0], at[1], at[2], at[3], r, abgr(*color)),
                }
                if *blend == Blend::Crosshair {
                    batch.invert_since(start);
                }
            }
            GuiDraw::AtlasSprite {
                sprite,
                x,
                y,
                w,
                h,
                color,
            } => {
                batch.sprite_scaled(
                    atlas,
                    sprite,
                    *x as f32,
                    *y as f32,
                    *w as f32,
                    *h as f32,
                    abgr(*color),
                );
            }
            GuiDraw::Blit {
                texture,
                blend: Blend::Textured,
                x,
                y,
                w,
                h,
                uv,
                src,
                tex,
                color,
            } => {
                let Some(mut r) = atlas.sprites.get(texture.as_ref()).copied() else {
                    continue;
                };
                let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                let u0 = r.u0 + du * uv[0] / tex[0] as f32;
                let v0 = r.v0 + dv * uv[1] / tex[1] as f32;
                r.u1 = r.u0 + du * (uv[0] + src[0] as f32) / tex[0] as f32;
                r.v1 = r.v0 + dv * (uv[1] + src[1] as f32) / tex[1] as f32;
                r.u0 = u0;
                r.v0 = v0;
                batch.rect_uv(*x as f32, *y as f32, *w as f32, *h as f32, r, abgr(*color));
            }
            GuiDraw::Blit { .. } | GuiDraw::Item { .. } => {}
            GuiDraw::Fill {
                x0,
                y0,
                x1,
                y1,
                color,
            } => {
                batch.fill(
                    atlas,
                    *x0 as f32,
                    *y0 as f32,
                    (x1 - x0) as f32,
                    (y1 - y0) as f32,
                    abgr(*color),
                );
            }
            GuiDraw::Text {
                text,
                x,
                y,
                color,
                shadow,
            } => {
                batch.text_styled(atlas, text, *x as f32, *y as f32, abgr(*color), *shadow);
            }
            GuiDraw::Push(ops) => {
                let mut m = pose;
                for op in ops {
                    pose_apply(&mut m, *op);
                }
                stack.push(m);
            }
            GuiDraw::Pop => {
                if stack.len() > 1 {
                    stack.pop();
                }
            }
        }
        if pose != IDENTITY {
            for v in &mut batch.vertices[start..] {
                let [x, y] = v.pos;
                v.pos = [
                    pose[0] * x + pose[2] * y + pose[4],
                    pose[1] * x + pose[3] * y + pose[5],
                ];
            }
        }
    }
}
