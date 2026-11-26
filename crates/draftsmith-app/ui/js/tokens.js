// 디자인 토큰 (M0 ⑦, 구현 노트 §81): style.css 의 :root 변수를 도면 위에 겹쳐 그리는 캔버스에서도 쓴다.
// 색·크기는 style.css 한 곳에서만 바꾼다. 밝은·어두운 화면이 바뀌면 다시 읽는다.

const cache = new Map();
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => cache.clear());

/** 토큰 값 (`--ov-hilite` 등) */
export function tok(name) {
  let v = cache.get(name);
  if (v === undefined) {
    v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    cache.set(name, v);
  }
  return v;
}
