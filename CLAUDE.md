# Afterimage

Windows용 이미지 뷰어. Rust + eframe/egui 0.37 (wgpu 백엔드).
최우선 목표는 **빨리 뜨고, 넘길 때 끊기지 않는 것**이다. 기능을 추가하다 이 둘이 나빠지면 안 된다.

## 명령

- 실행: `cargo run --release -- "D:\사진\example.jpg"`
- 빌드: `cargo build --release` → `target\release\afterimage.exe`
- 정리·검사: `cargo fmt`, `cargo clippy --release`
- debug 빌드에서도 의존 크레이트는 opt-level 2로 빌드된다 (`Cargo.toml`). 디코딩이 너무 느려지지 않게 하려는 것이니 지우지 않는다.
- 루트의 `afterimage.exe`는 배포용 사본이다. 소스를 고치면 `target\release\afterimage.exe`로 바꿔 둔다.
- 툴체인은 rustup의 `stable-x86_64-pc-windows-msvc`다. 새로 설치한 직후라면 `%USERPROFILE%\.cargo\bin`이 PATH에 없을 수 있다.

## 구조

| 파일 | 역할 |
| --- | --- |
| `src/main.rs` | 창 옵션, 글꼴 로드, 첫 이미지 디코딩을 창 생성 전에 시작 |
| `src/app.rs` | 화면 그리기, 입력, 줌/팬, 넘기기, 필름스트립, 정보 패널, 애니메이션 재생 |
| `src/loader.rs` | 폴더 탐색, 디코딩 워커, 프레임/썸네일/EXIF/히스토그램 |
| `src/spring.rs` | 스프링 애니메이션 |
| `src/sharp.rs`, `src/sharp.wgsl` | 100%를 넘긴 이미지를 그리는 wgpu 셰이더. 처음 확대할 때 파이프라인을 만든다 |
| `src/theme.rs` | 색과 글꼴 상수 |
| `src/wic.rs` | Windows 내장 코덱(WIC)으로 HEIC·AVIF 디코딩, `cfg(windows)` 전용 |
| `src/chrome.rs` | 상단 바를 창 제목 표시줄로 쓰기(창 서브클래싱, 끌기·크기 조절 판정), `cfg(windows)` 전용 |
| `src/i18n.rs` | 언어 감지와 UI 문구 표 |
| `src/update.rs` | GitHub Releases에서 자동 업데이트, `cfg(windows)` 전용 |
| `src/explorer.rs` | 열려 있는 탐색기 창의 보기 순서 읽기, 탐색기식 이름 비교(`StrCmpLogicalW`), `cfg(windows)` 전용 |
| `examples/sign.rs` | 배포 서명 도구 (키 만들기, `latest.txt` 만들기). 앱에는 들어가지 않는다 |
| `scripts/publish.ps1` | 초안 Release를 서명해서 공개 |
| `build.rs` | exe에 아이콘·버전 정보 넣기 (winresource) |
| `assets/` | `icon.svg`가 원본. png·ico는 여기서 다시 만든다 |

## 지켜야 할 규칙

### 성능
- 디코딩은 절대 UI 스레드에서 하지 않는다. 흐름: `Loader` 워커 → mpsc → `Viewer::poll()`에서 텍스처 업로드.
- 폴더 목록도 UI 스레드에서 만들지 않는다. `Loader::scan`이 백그라운드에서 만들어 `Done::Folder`로 보내고, 그 전까지는 연 파일 하나만 보여 준다.
- 미리 디코딩 순서는 현재, +1, −1, +2, −2. 전체 해상도 캐시는 ±3 범위만 유지한다. 넘기기가 처음↔끝을 순환하므로 이 둘도 폴더 양 끝에서 반대쪽으로 이어진다. 썸네일은 ±14 범위를 요청하고, 200개가 넘으면 ±60 밖을 버린다.
- `want_full` / `want_thumbs`는 대기열을 **교체**한다. 이미 디코딩 중인 작업은 `in_flight`로 중복을 막는다.
- GPU `max_texture_side`보다 큰 이미지는 디코딩 단계에서 한 번만 줄인다. `Meta.width/height`는 항상 원본 크기를 담는다 (100% 계산의 기준).
- 가만히 있을 때는 CPU를 쓰지 않는다. 무언가 움직일 때만 `request_repaint()`를 부르고, 애니메이션은 `request_repaint_after(다음 프레임까지)`로만 깨운다.

### 줌 좌표
- `scale`은 화면 point당 이미지 픽셀 수다. 100%는 `1 / pixels_per_point`이다.
- 맞춤(fit)은 100%를 넘겨 확대하지 않는다.
- 줌은 커서 아래 지점을 고정한 채로 한다 (`zoom_at`).
- 100%를 넘겨 확대할 때는 `sharp.rs`의 셰이더로 그린다. 픽셀은 네모로 두고 경계가 걸린 화면 픽셀만 섞어서, 어떤 배율에서도 칸 폭이 고르게 보인다. 정수 배율에서는 섞이는 픽셀이 없다. 100% 이하는 egui가 그린다(`tex_options`).

### 움직임
- 지속시간이나 easing을 쓰지 않고 `spring.rs`의 스프링만 쓴다. 아래 값은 디자인에서 확정한 것이라 임의로 바꾸지 않는다.
  - `SWIPE` 380/34: 넘기기, 필름스트립 스크롤
  - `ZOOM` 260/26: 줌, 팬
  - `PANEL` 420/24: 정보 패널
  - `THUMB` 520/26: 썸네일 선택
- 새로운 움직임이 필요하면 이 넷 중 하나를 재사용한다.
- 중간에 끊겨도 자연스럽게 이어지도록, 목표값(`to`)만 바꾸고 `x`와 `v`는 유지한다.
- 버튼 같은 UI 크롬은 스프링 없이 짧은 지수 스무딩(약 60ms)으로 나타나고 사라진다.

### UI 구현
- egui 위젯은 쓰지 않는다. `Painter`로 직접 그리고, 클릭 판정은 `ui.interact`로 한다.
- 나중에 등록한 `interact`가 위에 놓인다. 그래서 등록 순서를 캔버스 → 좌우 버튼 → 필름스트립 → 정보 패널 → 상단 바로 유지한다.
- 위젯 ID는 `ui.scope_id().with(...)`로 만든다. egui 0.37에서는 `Id::new`가 deprecated다.
- egui 0.37은 `App::ui(&mut self, ui, frame)` 시그니처를 쓴다. 예전 예제의 `update(ctx, frame)`와 다르다.
- 다음/이전(키, 좌우 버튼, 스와이프)은 끝에서 처음으로, 처음에서 끝으로 순환한다. 새 이미지는 누른 방향 쪽에서 밀려 들어온다(`go_from`). Home/End와 필름스트립 클릭은 순환하지 않는다.
- 시스템 제목 표시줄은 없고 상단 바가 그 역할을 한다(`chrome.rs`). 창 틀(그림자, 크기 조절, 스냅)은 시스템 것을 그대로 쓴다.
  - 창을 서브클래싱해서 `WM_NCCALCSIZE`로 캡션을 클라이언트 영역에 넣고, `WM_NCHITTEST`에서 상단 바의 빈 곳을 `HTCAPTION`, 맨 위 몇 픽셀을 `HTTOP`으로 돌려준다. 끌기, 더블클릭 최대화, 화면 가장자리 스냅, 시스템 메뉴는 그래서 Windows가 처리한다.
  - 상단 바에 버튼을 새로 넣으면 `draw_top_bar`의 `hot` 목록에도 넣는다. 빠뜨리면 그 자리가 끌기 영역으로 판정되어 클릭이 앱에 오지 않는다.
  - 최소화·최대화·닫기는 앱이 그린다(46pt 폭, 닫기 호버는 `CLOSE_HOVER`). 전체 화면에서는 단추를 숨기고, 끌기 영역도 0으로 둔다.
  - 최대화 버튼에 마우스를 올렸을 때 나오는 스냅 레이아웃 메뉴는 없다(Win+Z는 동작). 넣으려면 그 버튼을 `HTMAXBUTTON`으로 판정하고 NC 마우스 메시지를 직접 처리해야 한다.

## 디자인

- 디자인 캔버스: https://claude.ai/artifact/QL6yr1JwdzHfroQXQQHyvF (메인 뷰어, 모션 스펙, 앱 아이콘)
- 색은 `theme.rs` 상수만 쓴다.
  - 배경 `#0B0C0E`
  - 본문 텍스트 `#ECEDEF`, 보조 텍스트 `#9A9EA6`
  - 포인트 `#7AA2FF`
- 그라데이션은 쓰지 않는다. 아이콘도 마찬가지다.
- 앱 아이콘은 B안(뷰파인더 모서리 + 산 + 노란 해)을 단색으로 그린 것이다.
- UI 문구는 `src/i18n.rs` 표에만 쓴다. 한국어는 해요체로 쓴다. Windows 표시 언어가 한국어면 한국어, 그 밖에는 영어로 나온다(`AFTERIMAGE_LANG=en|ko`로 바꿀 수 있다). 언어를 추가할 때는 `main.rs`의 글꼴이 그 글자를 담는지 확인한다.
- 글꼴은 실행할 때 `C:\Windows\Fonts`에서 Segoe UI와 맑은 고딕을 읽어 온다 (번들하지 않음). `theme::strong()`은 Semibold/Bold 패밀리다.

## 형식 지원

- 자체 디코더: JPEG, PNG, GIF, WebP, BMP, TIFF, ICO. EXIF 회전을 적용한다.
- 애니메이션: GIF, WebP, APNG. 모든 프레임을 텍스처로 올린다.
  - 메모리 예산은 384MB이고, 넘으면 거기서 자른다.
  - 20ms 미만의 지연 시간은 100ms로 취급한다.
- WIC 경로 (`wic.rs`):
  - HEIC, HEIF, AVIF는 처음부터 WIC로 디코딩한다.
  - 자체 디코더가 실패한 파일도 WIC로 한 번 더 시도한다.
  - 코덱이 없으면(`WINCODEC_ERR_COMPONENTNOTFOUND`) Store 확장 설치 안내 문구를 보여준다.

## 폴더 순서

- 그 폴더를 보여 주는 탐색기 창이 있으면 창의 보기 순서(정렬 기준·방향·그룹)를 그대로 쓴다. 창이 여럿이면 그 파일이 선택된 창을 고른다.
- 창이 없거나 1.5초 안에 답이 없으면 `StrCmpLogicalW` 이름순으로 정렬한다. Windows가 아닌 환경에서는 natord를 쓴다.
- 탐색기 목록에 없는 파일(방금 생긴 파일 등)은 맨 뒤에 이름순으로 붙는다.

## 배포와 자동 업데이트

- 배포는 네 단계다.
  1. `Cargo.toml`의 `version`을 올리고 커밋한 뒤 `v<version>` 태그를 푸시한다. 태그와 버전이 다르면 워크플로가 실패한다.
  2. `.github/workflows/release.yml`이 빌드해서 초안 Release에 `afterimage.exe`를 올린다. 공개하지는 않는다.
  3. 워크플로가 끝나면 초안 Release에 영어로 릴리스 노트를 쓴다(`gh release edit v<version> --notes-file <파일>`). 자동 생성 노트는 PR만 모으므로 이 저장소에서는 비교 링크 한 줄만 나온다.
  4. 이 PC에서 `scripts\publish.ps1 <version>`을 실행한다. 노트가 비어 있으면 멈추고, 있으면 exe를 받아 서명한 `latest.txt`를 올리고 Release를 공개한다.
- 서명 키는 `%USERPROFILE%\.afterimage\release-signing.key`에만 있다. 커밋하거나 GitHub에 올리지 않는다. 잃어버리면 설치된 앱에 업데이트를 보낼 수 없으니 백업해 둔다. 공개 키는 `update.rs`의 `PUBLIC_KEY`이고, 바꾸면 설치된 앱이 업데이트를 못 받는다.
- `latest.txt`는 `<version> <sha256> <서명>` 한 줄이다. 서명이 없거나 틀리면 앱은 업데이트하지 않는다. 0.3.0 이하는 앞의 두 칸만 읽으니, 세 번째 칸을 추가해도 이 버전들은 그대로 업데이트된다.
- 저장소: https://github.com/Hoyeom/afterimage (공개). `*.exe`는 커밋하지 않는다.
- `update.rs`는 실행마다 한 번, 3초 뒤 백그라운드에서 `latest.txt`를 읽는다. GitHub의 `releases/latest/download`가 가끔 504를 돌려주므로 20초 간격으로 세 번까지 시도하고, 그래도 실패하면 조용히 넘어간다. 시작 속도에 영향을 주면 안 된다.
- 교체는 실행 중인 exe를 `.exe.old`로 옮기고 새 exe를 그 자리에 두는 방식이다. 다음 실행 때 적용되고 `.exe.old`도 그때 지운다.
- 경로에 `target` 폴더가 있으면(개발 빌드) 건너뛴다. `AFTERIMAGE_NO_UPDATE`로 끌 수 있다.
- `latest.txt`와 exe의 파일 이름·형식을 바꾸면 이미 배포된 버전이 업데이트를 못 받는다.
- exe 자체의 코드 서명은 없다(SmartScreen 경고가 뜰 수 있다). 업데이트 서명은 GitHub 계정이 뚫려도 위조를 막지만, 서명하는 PC가 뚫리거나 빌드가 변조된 채 서명하면 막지 못한다.

## 아직 확인 못 한 것 / 할 일

- HEIC는 Windows 인코더로 만든 샘플만 열어 봤다. 회전값 6을 넣은 샘플도 바르게 보인다.
  - 휴대폰으로 찍은 실제 HEIC는 아직 못 봤다. HEIF 자체 회전 정보(irot)와 EXIF 회전값이 다르게 처리될 수 있는데, 현재 WIC 경로는 회전을 따로 적용하지 않는다.
- AVIF는 실제로 열어 본 적이 없다. Windows에 AVIF 인코더가 없어 샘플을 따로 구해야 한다.- RAW 미지원. 파일 안에 들어 있는 JPEG 미리보기를 꺼내 쓰는 방식으로 넣을 예정이다.
- Windows 기본 프로그램 등록(파일 연결)이 아직 없다.
- 큰 JPEG의 썸네일은 전체를 디코딩한 뒤 줄인다. 느리면 WIC 스케일러나 DCT 단계 축소 디코딩을 검토한다.
