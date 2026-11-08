# Afterimage

빠르게 열리고, 넘길 때 끊기지 않는 Windows용 이미지 뷰어.

## 실행

`afterimage.exe <이미지 파일>` 또는 이미지 파일을 창으로 끌어다 놓기.
같은 폴더의 이미지를 함께 불러와요. 그 폴더를 연 탐색기 창이 있으면 창의 정렬 순서를 그대로 따르고, 없으면 탐색기와 같은 이름순으로 보여 줘요.

[Releases](https://github.com/Hoyeom/afterimage/releases/latest)에서 `afterimage.exe`를 받아 바로 실행하면 돼요. 코드 서명이 없어서 처음 실행할 때 Windows 경고가 뜰 수 있어요.

## 단축키

| 키 | 동작 |
| --- | --- |
| ← / → , PageUp / PageDown, Space / Backspace | 이전 / 다음 |
| Home / End | 처음 / 마지막 |
| 마우스 휠, Ctrl+휠, 핀치 | 커서 위치 기준 줌 |
| + / − | 확대 / 축소 |
| 0 | 화면에 맞춤 |
| 1 | 실제 크기 (100%) |
| 더블클릭 | 맞춤 ↔ 100% |
| 드래그 | 확대 상태에서는 이동, 맞춤 상태에서는 좌우로 밀어 넘기기 |
| I | 정보 패널 |
| P | 애니메이션 일시정지 / 재생 |
| F / F11 | 전체 화면 |
| Ctrl+O | 파일 열기 |
| Esc | 전체 화면 해제 → 패널 닫기 → 종료 |

## 구조

- `src/loader.rs` — 폴더 탐색, 백그라운드 디코딩 워커, 앞뒤 2장 미리 디코딩, 썸네일, EXIF, 히스토그램
- `src/spring.rs` — 스프링 애니메이션 (모션 스펙 값: 넘기기 380/34, 줌 260/26, 패널 420/24, 썸네일 520/26)
- `src/app.rs` — 화면 그리기와 입력 처리
- `src/theme.rs` — 색과 글꼴
- `src/wic.rs` — Windows 내장 코덱으로 HEIC·AVIF 디코딩
- `src/explorer.rs` — 탐색기 창의 정렬 순서 읽기
- `src/chrome.rs` — 상단 바를 창 제목 표시줄로 쓰기
- `src/update.rs` — 자동 업데이트
- `assets/` — 앱 아이콘 (뷰파인더 B안)

렌더링은 eframe(egui) + wgpu. 디코딩은 UI 스레드 밖에서 하고, GPU 최대 텍스처 크기를 넘는 이미지는 디코딩 단계에서 한 번만 줄여요.

## 자동 업데이트

앱을 켤 때마다 백그라운드에서 GitHub Releases의 최신 버전을 확인해요. 새 버전이 있으면 받아서 SHA-256을 검증한 뒤 실행 파일을 교체하고, 다음에 켤 때 적용돼요. 이 과정에서 github.com에 접속하지만 사용자 정보나 이미지는 보내지 않아요.

끄려면 환경 변수 `AFTERIMAGE_NO_UPDATE=1`을 설정하세요. 쓰기 권한이 없는 폴더(`Program Files` 등)에서는 교체하지 않고 새 버전이 있다고만 알려 줘요.

## 빌드

Rust가 없다면 https://rustup.rs 에서 설치(기본값 그대로)한 뒤:

```
cargo run --release -- "D:\사진\example.jpg"
```

결과물: `target\release\afterimage.exe`

## 지원 형식

JPEG, PNG, GIF, WebP, BMP, TIFF, ICO — 자체 디코더
GIF · WebP · APNG 애니메이션 — 프레임 지연 시간대로 재생 (메모리 보호를 위해 약 384MB까지)
HEIC · HEIF · AVIF — Windows 내장 디코더(WIC) 사용. 아래 Microsoft Store 확장이 있어야 열려요.
- HEIC: 'HEIF 이미지 확장' + 'HEVC 비디오 확장'
- AVIF: 'AV1 비디오 확장'

RAW는 다음 단계에서 추가 예정.

## 라이선스

[MIT](LICENSE)
