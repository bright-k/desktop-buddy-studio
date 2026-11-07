# Desktop Buddy Studio

설정과 리소스로 입력 반응형 데스크톱 캐릭터 프로그램을 정의하고, 각각 독립 실행 Windows·macOS 앱으로 빌드하는 제작 도구입니다.

- 기반 기술: **Tauri + Rust**
- 공통 기능: 다른 앱 작업 중 전역 키보드·마우스 반응, 입력 속도별 애니메이션, 특정 키·한글 키 순서 이스터에그, 투명 오버레이, 위치·크기 조정
- 최초 대상: Windows 11 25H2 이상 x64, macOS 15 이상 arm64
- 앱별 구성: 제품 이름·아이콘·식별자·캐릭터·애니메이션·입력 동작
- 배포 방향: 공식 인증서·공증 불필수, 필요한 로컬 서명·셀프 서명 허용

현재는 M0~M3 구현·검증을 진행 중입니다. 프로젝트 검증·생성, 공통 Rust 행동 엔진, OS 입력 어댑터, Tauri Player와 독립 앱 Builder를 구현하고 단계별로 검증하고 있습니다. **MVP 출시 완료 상태는 아닙니다.** 실제 통과·미검증 항목은 [개발 검증 기록](docs/validation/2026-11-07-development.md)에 둡니다.

로컬 웹 Studio와 Builder CLI를 제공합니다. Studio에서 프로젝트·리소스·이벤트·타이밍을 편집하고 앱으로 빌드할 수 있습니다. 생성 앱의 사용자는 개발 도구가 필요 없습니다. 한글 키워드는 두벌식 물리 키 순서로 판정하며 실제 입력 문자열·입력 이력은 읽거나 저장하지 않습니다.

## Studio 실행

```sh
npm install
npm run studio
```

macOS에서는 [Studio.command](Studio.command)를 더블클릭할 수도 있습니다. 프로젝트별 원본·저장 이력·빌드 결과는 `.data/projects/`에 모입니다. [Studio 사용 안내](docs/studio.md)를 참고하세요. 현재 Mac에서는 개발용 앱·DMG와 ZIP을 빌드합니다.

프로젝트 폴더와 브라우저 주소는 `com.example.typingcat` 같은 패키지 ID를 사용합니다. 임시 편집을 자동 보관하고 새로고침 시 복원하며, 검증하여 저장한 구성이 미리보기·빌드에 적용됩니다. [사용성 개선 기록](docs/studio-ux-review.md)에 편집·오류 복구·반응형 화면 개선을 정리했습니다.

빌드 탭에서 GitHub 저장소를 연결하고 워크플로를 준비하면 **업로드하고 두 플랫폼 빌드**로 macOS·Windows 앱을 함께 만듭니다. 저장된 정의·참조 리소스·고정 엔진을 별도 브랜치에 업로드하고 진행 상황과 검증한 결과를 프로젝트 안에 가져옵니다. 공개 저장소에서는 업로드한 원본도 공개됩니다. 현재 OS만 만드는 로컬 빌드와 프로젝트 내보내기도 유지합니다. [Actions 연동 안내](docs/studio-actions.md)를 참고하세요.

## 개발 환경과 실행

Rust 1.99.1, Node.js 22 이상, 해당 OS의 Tauri 빌드 도구가 필요합니다. Rust 버전은 rust-toolchain.toml, 웹 의존성은 package-lock.json에 고정합니다.

필요한 Rust는 `rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy`로 직접 준비합니다. `buddy doctor`는 고정 버전과 대상 표준 라이브러리를 검사하며, 도구가 없을 때 자동 설치하지 않고 준비 안내와 종료 코드 1을 반환합니다.

```sh
npm ci
node scripts/cargo.mjs run --locked -p buddy-cli -- doctor
node scripts/cargo.mjs run --locked -p buddy-cli -- validate examples/typing-cat
npm run player:dev
```

개발 실행은 별도 dev 설정에서 macOS 14 이상을 허용합니다. 이것을 정식 macOS 15 이상 지원 검증으로 취급하지 않습니다. 입력 권한은 Player 설정에서 사용자가 요청하며, 권한이 없으면 idle과 설정 기능을 유지합니다.

```sh
npm run check
node scripts/cargo.mjs run --locked -p buddy-cli -- new examples/my-buddy --name "My Buddy"
```

독립 앱 Builder는 CLI와 동일 소스에서 만든 템플릿 팩이 필요합니다. 템플릿은 CLI에 내장된 manifest/hash로 검증하며, 소스가 바뀌면 CLI와 팩을 함께 다시 만듭니다. 아래 development 빌드는 출시 검증용 산출물이 아닙니다.

```sh
node scripts/cargo.mjs run --locked -p buddy-cli -- template-pack --source . --output artifacts/player-template-pack
node scripts/cargo.mjs run --locked -p buddy-cli -- build examples/typing-cat --template-pack artifacts/player-template-pack --development --output artifacts/builds
```

CLI·공통 미리보기 Player·검증된 템플릿 팩을 한 디렉토리로 묶는 개발 명령도 제공합니다. 종속성 고지와 필요한 소스 아카이브도 함께 포함합니다. 생성된 묶음 안의 `buddy`는 `new`, `validate`, `doctor`, `preview`, `build`를 제공합니다. macOS 14 개발용 묶음은 아래처럼 만들며, 정식 지원 OS에서는 `--development`를 빼고 검증합니다. 패키징 성공은 외부 기기 설치·입력 검증을 대신하지 않습니다.

```sh
npm run package:cli -- --development --output artifacts/cli/local-development
artifacts/cli/local-development/buddy preview examples/typing-cat --development
```

[문서 시작하기](docs/README.md) · [요구사항](docs/product-requirements.md) · [아키텍처](docs/architecture.md) · [구현 계획](docs/implementation-plan.md)

저장소: [bright-k/desktop-buddy-studio](https://github.com/bright-k/desktop-buddy-studio)

성능 계측용 프로젝트는 [examples/performance-30fps](examples/performance-30fps)에 있습니다. 설정의 **반응 성능 진단**에서 집계를 초기화·조회하고 JSON으로 저장할 수 있습니다. 측정 조건과 제외 표본 해석은 [시험 전략](docs/testing-strategy.md#6-성능-측정-방법)을 따릅니다. 실제 지원 OS의 성능 달성 검증은 아직 완료하지 않았습니다.
