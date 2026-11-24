//! DXF·DWG 가져오기 (acadrust → 도면 모델).
//!
//! 지금 가져오는 것: 선·폴리선(LW·2D)·원·호·한 줄 문자·여러 줄 문자(위치만).
//! 블록 참조(INSERT)·해치·치수 등은 [`Drawing::skipped`] 에 개수만 남긴다 — M0 ② 에서 늘린다.

use std::path::Path;

use acadrust::{CadDocument, DwgReader, DxfReader, EntityType};
use thiserror::Error;

use crate::drawing::{Drawing, Entity, Shape};
use crate::geom::Vec2;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("파일을 열 수 없습니다: {0}")]
    Io(#[from] std::io::Error),
    #[error("도면을 읽지 못했습니다: {0}")]
    Cad(String),
}

/// 파일 머리말로 DWG 인지 본다 (`AC10..` 으로 시작). 아니면 DXF 로 읽는다.
fn is_dwg(path: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut head = [0u8; 4];
    let n = std::fs::File::open(path)?.read(&mut head)?;
    Ok(n == 4 && &head[..2] == b"AC" && head[2].is_ascii_digit())
}

/// DXF 또는 DWG 파일을 읽는다.
pub fn read_file(path: impl AsRef<Path>) -> Result<Drawing, ImportError> {
    let path = path.as_ref();
    let doc = if is_dwg(path)? {
        DwgReader::from_file(path)
            .and_then(|mut r| r.read())
            .map_err(|e| ImportError::Cad(e.to_string()))?
    } else {
        DxfReader::from_file(path)
            .and_then(|r| r.read())
            .map_err(|e| ImportError::Cad(e.to_string()))?
    };
    Ok(from_document(&doc))
}

fn v2(v: &acadrust::types::Vector3) -> Vec2 {
    Vec2::new(v.x, v.y)
}

fn rgb(c: &acadrust::types::Color) -> Option<[u8; 3]> {
    c.rgb().map(|(r, g, b)| [r, g, b])
}

/// acadrust 문서를 도면 모델로 바꾼다 (모형 공간 객체만).
pub fn from_document(doc: &CadDocument) -> Drawing {
    let mut d = Drawing {
        code_page: doc.header.code_page.clone(),
        ..Default::default()
    };

    for l in doc.layers.iter() {
        let i = d.layer_index(&l.name) as usize;
        let layer = &mut d.layers[i];
        if let Some(c) = rgb(&l.color) {
            layer.color = c;
        }
        layer.visible = !l.flags.frozen && !l.is_off();
    }

    for e in doc.entities() {
        let (common, shape) = match e {
            EntityType::Line(x) => (
                &x.common,
                Shape::Line {
                    a: v2(&x.start),
                    b: v2(&x.end),
                },
            ),
            EntityType::LwPolyline(x) => (
                &x.common,
                Shape::Polyline {
                    points: x
                        .vertices
                        .iter()
                        .map(|v| (Vec2::new(v.location.x, v.location.y), v.bulge))
                        .collect(),
                    closed: x.is_closed,
                },
            ),
            EntityType::Polyline2D(x) => (
                &x.common,
                Shape::Polyline {
                    points: x
                        .vertices
                        .iter()
                        .map(|v| (v2(&v.location), v.bulge))
                        .collect(),
                    closed: x.is_closed(),
                },
            ),
            EntityType::Circle(x) => (
                &x.common,
                Shape::Circle {
                    center: v2(&x.center),
                    radius: x.radius,
                },
            ),
            EntityType::Arc(x) => (
                &x.common,
                Shape::Arc {
                    center: v2(&x.center),
                    radius: x.radius,
                    start: x.start_angle,
                    end: x.end_angle,
                },
            ),
            EntityType::Text(x) => (
                &x.common,
                Shape::Text {
                    at: v2(&x.insertion_point),
                    height: x.height,
                    rotation: x.rotation,
                    value: x.value.clone(),
                },
            ),
            EntityType::MText(x) => (
                &x.common,
                Shape::Text {
                    at: v2(&x.insertion_point),
                    height: x.height,
                    rotation: x.rotation,
                    value: x.value.clone(),
                },
            ),
            other => {
                *d.skipped.entry(kind_name(other)).or_insert(0) += 1;
                continue;
            }
        };
        let layer = d.layer_index(&common.layer);
        d.entities.push(Entity {
            layer,
            color: rgb(&common.color),
            shape,
        });
    }
    d
}

/// 객체 종류 이름 (`Insert`, `Hatch` …) — Debug 출력의 첫 낱말
fn kind_name(e: &EntityType) -> String {
    let s = format!("{e:?}");
    s.split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("?")
        .to_string()
}
