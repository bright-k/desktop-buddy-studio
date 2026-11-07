# Studio·추가 이미지·직접 드래그 확장 검증

환경: macOS 14.4.1 arm64, Node 24.18.0, Rust 1.99.1. 기준일: 2026-11-07.

## 자동 검증

`npm run check` 통과. Rust 81개, Canvas renderer 5개, Studio/URL/이전/임시 편집/내보내기 10개, 합계 96개 테스트가 통과했다. Player/Studio TypeScript·Vite 빌드와 workspace clippy `-D warnings`를 포함한다. 로컬 전체 로그는 `artifacts/studio-2026-11-07/check.log`에 남겼다.

- 기존 PNG 프로젝트 호환, PNG/JPEG/WebP/BMP 디코딩, GIF 프레임·원본 지연·정규화 결과를 검증했다.
- GIF 80/240ms 프레임의 320ms 일회성 종료, 렌더러의 프레임 교체·사각형 좌표·재시작과 정적 유휴 중단을 검증했다.
- 기본 250ms 유휴, 사용자 지정 타이밍, durationMs·stopOnRelease, 키 유지 sustain·drag 우선 동작과 해제를 검사했다.
- 숨김 폴더 아래 두 프로젝트의 리소스 격리, 불변 저장 버전, 충돌 거부, 복제의 새 앱 ID, 이전 버전 복원, 보관/복원을 검증했다.
- 인증·동일 origin·경로 탈출·symlink 차단, 손상 리소스 거부, 작업 폴더 단일 소유권, 실제 Rust 모의 입력·키워드 컴파일러 경로를 검사했다.
- URL의 프로젝트·탭·동작·이벤트 상태 왕복과 잘못된 query 복구를 검사했다.
- 패키지 ID 폴더 생성·충돌·기존 UUID 이전·중단 복구·기존 다운로드 별칭, 임시 편집의 재시작 복원·다른 탭 충돌, 저장과 임시 편집 분리, 동일 버전의 중복 빌드 재사용을 검증했다.
- 이동용 묶음의 GIF 원본·내부 projectId·앱 identifier·content hash 보존, 미참조 파일·임시 편집 제외, 오래된 revision 요청 거부를 검사했다.
- `prepare-notices --check` 통과: Rust 290개, 웹/도구 4개, 원문 소스 아카이브 17개.

## 브라우저 검증

로컬 웹 Studio에서 새 프로젝트 생성, GIF 업로드, celebrate의 GIF 교체, PNG의 (256,256,256,256) 영역 지정, 드래그→walk 이벤트 추가, 빠른 타이밍 프리셋 저장, 실제 Rust 엔진 테스트, 프로젝트 복제를 실행했다. 키워드 GIF 종료 후 대기 복귀와 빌드 보고서 다운로드도 확인했다.

두 프로젝트 중 이전 프로젝트를 선택하고 리소스 탭에서 새로고침해 project/tab/clip/event 선택이 유지되는 것을 확인했다. 1440px·390px 폭에서 문서 가로 넘침이 없고, 두 이미지가 리소스 영역 안에 들어가며 설명은 이미지 아래에 놓이는 것을 DOM 좌표와 화면으로 확인했다. 증거 화면은 `artifacts/studio-2026-11-07/resources-desktop.jpg`에 둔다.

## 실제 앱 빌드

Studio Demo 프로젝트에 GIF·PNG 영역·drag 이벤트·150ms 유휴 설정을 넣고 웹의 앱 빌드로 개발용 .app·DMG를 생성했으며 Studio가 ZIP을 추가했다. 완료 이력과 다운로드 링크를 확인했다.

- 프로젝트: `.data/projects/io.desktopbuddy.app.p7120656502554e84bf8adf54abdc6ad2/` (이전 관리 ID: `25898176-222c-4753-a04f-a3e7e3906ad8`)
- 결과: `builds/buddy-25898176-0.1.0-9bf15c3b-7c44-4842-b31b-dfc00b6cc9ec/`
- 프로젝트 hash: `f9e4c1c2d7e40f11f4e1325c27c6bc2f6d8b03bf680b5cc32feb245068ce66fd`
- 검증한 고정 팩 hash: `8be23e7e23734f3497c66e5f90df3a87676c925e067bf021719800b1417bbf3b`
- codesign `--verify --deep --strict`, ZIP `unzip -tq` 통과. 앱 안에 원본 GIF·영역·타이밍·drag 정의와 라이선스가 들어 있음을 확인했다.

해당 앱은 네이티브 트레이 상태·설정 창 전면 표시·즉시 드래그·사용자 타이밍 변경 코드가 포함된 팩으로 빌드했다. 실행 중인 이전 Typing Cat 앱을 교체하거나 입력 모니터링 권한을 변경하지 않았다. 사용자가 수동 시험을 맡기로 했으므로 새 앱의 실제 입력·창 포커스·드래그 조작을 통과로 표시하지 않는다. 현재 지원 기준 OS, Windows 실기, 외부 설치·장시간 성능 검증은 이 기록에 포함하지 않는다.

## 패키지 ID·UX 추가 검증

실제 기존 프로젝트 두 개의 이전 전후 원본 hash·내부 projectId·앱 identifier가 동일함을 확인했다. 기존 UUID URL은 canonical package ID 주소로 바뀌고 이전 다운로드도 연결됐다. 이전 전 상태는 `artifacts/studio-2026-11-07/storage-before.json`에 남겼다.

브라우저에서 com.example.typingcat 프로젝트를 생성했다. 180ms 타이밍을 임시 편집한 후 새로고침 복원, Cmd+S 저장, 잘못된 FPS 31 입력의 지속되는 오류 안내와 값 보존, FPS 수정 후 정상 저장, 정상 GIF·손상 PNG 동시 업로드의 개별 처리, 연속 빌드 클릭이 한 작업만 만드는 것을 확인했다. 저장 후 화면에서는 변경 취소·저장 후 미리보기 버튼이 실제로 숨겨진다.

해당 프로젝트의 저장된 구성은 180ms 중단·celebrate GIF·drag→walk이며 hash는 `c3ef6e736dc746b4d30f012d32d55c5adab71da161ab62999c3b3502c0145762`다. 실제 웹 빌드 결과는 `.data/projects/com.example.typingcat/builds/typing-cat-lab-0.1.0-28490769-27d3-4917-8a6f-a5e13aae221c/`에 있다. codesign 검사·ZIP 무결성과 앱 내부 정의를 확인했다.

플랫폼 카드·현재 호스트 버튼·이력의 대상 구분을 브라우저에서 확인했고, 저장한 프로젝트 내보내기를 눌러 `com.example.typingcat-84e6b8f0.tar.gz`를 다운로드했다. `.github/workflows/build-project.yml`은 동일 커밋의 정의를 두 OS runner로 전달한다. 공통 `scripts/build-project.mjs`의 macOS 개발용 앱·DMG 생성은 로컬에서 실행했다. 로그는 `artifacts/studio-2026-11-07/platform-build.log`에 있다. 원격 CI 실행과 Windows 결과물의 컴파일·실행·설치는 수행하지 않았다.

최종 화면은 1440·1100·768·390px 폭에서 문서 가로 폭이 viewport를 넘지 않고 미리보기 캔버스가 정사각형을 유지했다. 이미지 두 개는 부모 영역 안에 들어갔으며 좁은 화면의 플랫폼 카드·다운로드 버튼은 줄바꿈됐다. 리소스 보관함에서 이미 참조한 파일은 ‘사용 중’으로 비활성화된다. 브라우저 console 오류는 없었다. 화면 증거는 resources-desktop/mobile.jpg와 build-platforms-desktop/mobile.jpg에 보관했다.

최종 공통 빌드 스크립트는 앱·DMG·source.json·보고서만 담은 `artifacts/project-macos-arm64.tar.gz`를 생성했다(25,742,355 bytes). Cargo 캐시·이전 빌드를 제외했고, source.json의 hash는 Studio의 저장 구성과 일치했다. 포함 앱의 codesign `--verify --deep --strict`와 잘못된 호스트에서 Windows 빌드 요청 거부를 확인했다. `artifacts/studio-2026-11-07/platform-summary.json`에 묶음 SHA-256·검사 결과를 남겼다. 두 workflow 파일은 YAML 구문 검사를 통과했다.
