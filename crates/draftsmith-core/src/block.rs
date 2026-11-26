//! 블록 만들기·참조·펼치기 (M2, 구현 노트 §30).
//!
//! 블록 참조(`Shape::Insert`)는 도면에 한 객체로 남고, 화면·출력·내보내기에는 `archgen::with_arch` 가 펼친 객체를
//! 대신 보인다 (건축 객체 선과 같은 길). 블록 안 레이어 「0」 객체는 참조의 레이어를, 색이 없으면 참조의 색을 따른다.

use crate::drawing::{Block, Drawing, Entity, Shape};
use crate::geom::Vec2;

/// 블록 안 블록 깊이 한도 (자기 자신을 품은 블록 막기)
const MAX_DEPTH: usize = 8;

/// 블록 참조 하나를 펼친 객체들
pub fn expand(d: &Drawing, e: &Entity) -> Vec<Entity> {
    let mut out = Vec::new();
    expand_into(d, e, 0, &mut out);
    out
}

fn expand_into(d: &Drawing, e: &Entity, depth: usize, out: &mut Vec<Entity>) {
    let Shape::Insert {
        block,
        at,
        scale,
        rotation,
        attrs,
    } = &e.shape
    else {
        out.push(e.clone());
        return;
    };
    if depth >= MAX_DEPTH {
        return;
    }
    let Some(b) = d.blocks.iter().find(|b| &b.name == block) else {
        return;
    };
    let t = Block::transform(*at, *scale, *rotation);
    // 속성값 글자 (참조의 레이어·색으로, §46)
    for a in attrs {
        out.push(Entity {
            id: 0,
            layer: e.layer,
            color: e.color,
            aci: e.aci,
            linetype: None,
            weight: None,
            shape: Shape::Text(a.text.clone()).transformed(&t),
        });
    }
    let zero = d
        .layers
        .iter()
        .position(|l| l.name == "0")
        .map(|i| i as u32);
    for x in &b.entities {
        let mut y = x.clone();
        if Some(y.layer) == zero {
            y.layer = e.layer;
            if y.color.is_none() {
                y.color = e.color;
                y.aci = e.aci;
            }
        }
        y.shape = x.shape.transformed(&t);
        expand_into(d, &y, depth + 1, out);
    }
}

/// 블록 정의 만들기: 객체들을 기준점이 원점이 되게 옮겨 담는다
pub fn define(name: &str, items: &[Entity], base: Vec2) -> Block {
    let t = crate::geom::Affine::translate(-base.x, -base.y);
    Block {
        name: name.to_string(),
        entities: items
            .iter()
            .map(|e| Entity {
                shape: e.shape.transformed(&t),
                ..e.clone()
            })
            .collect(),
        attdefs: Vec::new(),
    }
}

/// 블록 이름이 쓸 수 있는가 (비지 않고, 이미 없고)
pub fn check_name(d: &Drawing, name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("블록 이름을 넣으세요".into());
    }
    if d.blocks.iter().any(|b| b.name == n) {
        return Err(format!("「{n}」 블록이 이미 있습니다"));
    }
    Ok(())
}

/// 블록 참조를 모두 그 자리에서 펼친 도면 (블록을 모르는 곳 — CLI 보기·측정·시험용)
pub fn flatten(d: &Drawing) -> Drawing {
    if !d
        .entities
        .iter()
        .any(|e| matches!(e.shape, Shape::Insert { .. }))
    {
        return d.clone();
    }
    let mut out = d.clone();
    out.entities = d.entities.iter().flat_map(|e| expand(d, e)).collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Editor;
    use crate::geom::Affine;

    fn line(ax: f64, ay: f64, bx: f64, by: f64) -> Shape {
        Shape::Line {
            a: Vec2::new(ax, ay),
            b: Vec2::new(bx, by),
        }
    }

    #[test]
    fn make_insert_explode() {
        let mut ed = Editor::new(crate::archgen::new_drawing());
        let _ = ed.add_shapes(vec![
            line(100.0, 100.0, 200.0, 100.0),
            line(100.0, 100.0, 100.0, 200.0),
            line(0.0, 0.0, 1.0, 1.0),
        ]);
        // 처음 둘로 블록, 기준점 (100,100)
        let i = ed
            .make_block("의자", &[0, 1], Vec2::new(100.0, 100.0))
            .unwrap();
        assert_eq!(ed.d.entities.len(), 2);
        assert!(matches!(ed.d.entities[i].shape, Shape::Insert { .. }));
        // 펼치면 원래 자리
        let v = crate::archgen::with_arch(&ed.d).unwrap();
        assert!(
            v.entities
                .iter()
                .any(|e| e.shape == line(100.0, 100.0, 200.0, 100.0))
        );
        // 하나 더 넣고 90° 돌리기
        let j = ed
            .insert_block(
                "의자",
                Vec2::new(1000.0, 0.0),
                2.0,
                std::f64::consts::FRAC_PI_2,
            )
            .unwrap();
        let ex = expand(&ed.d, &ed.d.entities[j]);
        match &ex[0].shape {
            Shape::Line { a, b } => {
                assert!(
                    a.dist(Vec2::new(1000.0, 0.0)) < 1e-9
                        && b.dist(Vec2::new(1000.0, 200.0)) < 1e-9,
                    "{a:?} {b:?}"
                );
            }
            s => panic!("{s:?}"),
        }
        // 대칭 이동해도 펼친 모양이 같은 변환을 받은 것과 같다
        let m = Affine::scale(-1.0, 1.0);
        ed.transform(&[j], &m);
        let ex2 = expand(&ed.d, &ed.d.entities[j]);
        let want = ex[0].shape.transformed(&m);
        match (&ex2[0].shape, &want) {
            (Shape::Line { a, b }, Shape::Line { a: c, b: e }) => {
                assert!(a.dist(*c) < 1e-9 && b.dist(*e) < 1e-9);
            }
            _ => unreachable!(),
        }
        // 펼치기
        // 펼치기: 선 하나 + 참조 둘 → 참조 하나가 선 둘로
        ed.explode(j);
        assert_eq!(ed.d.entities.len(), 4);
        assert!(ed.undo());
        // 속성: 더하고 고치면 펼친 글자가 참조와 함께 돈다 (§46)
        ed.add_attr(j, "no", "1", 250.0).unwrap();
        assert!(ed.add_attr(j, "NO", "2", 250.0).is_err());
        ed.set_attr(j, "NO", "A-01").unwrap();
        let tx: Vec<_> = expand(&ed.d, &ed.d.entities[j])
            .into_iter()
            .filter_map(|e| match e.shape {
                Shape::Text(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(tx.len(), 1);
        assert_eq!(tx[0].value, "A-01");
        assert!((tx[0].height - 250.0).abs() < 1e-9, "{}", tx[0].height);
        // 속성 정의로 저장하면 새 참조에 기본값으로 붙는다 (§49)
        assert_eq!(ed.define_attrs(j).unwrap(), 1);
        let k = ed
            .insert_block("의자", Vec2::new(0.0, 5000.0), 1.0, 0.0)
            .unwrap();
        match &ed.d.entities[k].shape {
            Shape::Insert { attrs, .. } => assert_eq!(attrs[0].text.value, "A-01"),
            s => panic!("{s:?}"),
        }
        assert!(ed.undo() && ed.undo());
        // 다시 펼치기 (위 되돌리기 앞 상태와 같은 개수 + 속성 글자 하나)
        ed.explode(j);
        assert_eq!(ed.d.entities.len(), 5);
        assert!(ed.undo());
        assert_eq!(ed.d.entities.len(), 3);
        assert!(ed.make_block("의자", &[0], Vec2::default()).is_err());
    }
}
