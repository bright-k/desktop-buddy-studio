# 기술 선택 비교

검토일: 2026-11-07 / 결론: **Tauri + Rust 채택**. 사용자 결정은 [ADR 0001](decisions/0001-tauri-rust.md)에 기록했다. 아래 적합도는 제품 요구에 대한 설계 판단이며 벤치마크 결과가 아니다.

## 1. 무엇을 비교하는가

일반적인 설정 화면보다 다음 항목이 선택을 좌우한다.

1. 다른 앱을 사용하는 동안 키보드·마우스를 관찰할 수 있는가.
2. 투명·무테 창이 포커스를 빼앗지 않고 클릭을 통과시키는가.
3. 상주 앱의 CPU·GPU·메모리·전력을 관리할 수 있는가.
4. 한 공통 런타임에서 이름·아이콘·설정이 다른 독립 앱을 반복 생성할 수 있는가.
5. 제작자와 배포받은 사용자의 설치 부담이 어느 정도인가.

**TypeScript는 프레임워크 대안이 아니라 화면 언어 선택**이다. Tauri도 TypeScript 화면을 사용할 수 있다. 실질적인 비교는 Electron의 Chromium/Node 기반 호스트와 Tauri의 Rust/시스템 WebView 기반 호스트다. 각각의 프로세스 구조는 [Electron](https://www.electronjs.org/docs/latest/tutorial/process-model), [Tauri](https://v2.tauri.app/concept/process-model/) 공식 문서에 설명되어 있다.

## 2. 대안 비교

| 선택지 | 이 제품에서의 장점 | 구현·운영 비용 | 판단 |
| --- | --- | --- | --- |
| **Tauri 2 + Rust + 웹 화면** | Rust에서 프로젝트 검증·빌드·입력·창 관리를 공유하고 설정 화면을 웹 기술로 만들 수 있다. | Rust와 각 OS WebView를 함께 다뤄야 한다. 전역 입력은 별도 네이티브 계층이 필요하다. | **채택**. 작은 상주 Runtime과 재사용 Builder를 우선하는 현재 방향에 적합하다. |
| Electron + TypeScript | Chromium을 기준으로 화면 동작을 맞추기 쉽고 JavaScript 중심으로 개발할 수 있다. 창·트레이·패키징 경로가 갖춰져 있다. | 런타임 포함 비용, 네이티브 입력 모듈의 패키징·버전 검증이 필요하다. | 웹 개발 경험과 렌더러 일관성이 가장 중요한 팀에는 여전히 유력하다. |
| Flutter + Dart + 네이티브 연동 | 편집기 UI와 그래픽 표현에 적합하고 Windows·Mac용 앱을 만들 수 있다. | 오버레이·전역 입력의 플랫폼 연동, Dart와 네이티브 코드 운영이 추가된다. | 복잡한 제작 UI나 모바일 확장이 우선일 때 재검토한다. |
| Qt + C++/QML | 창과 그래픽을 네이티브 영역에서 제어할 수 있고 투명 창 구현 경로가 있다. | C++·QML·패키징 체계와 OS 입력 어댑터를 유지해야 한다. | Qt 경험이 있는 팀에는 적합하나 현재 프로젝트에서 전환 이점이 충분하지 않다. |
| Godot + GDScript/C# + 네이티브 확장 | 캐릭터 애니메이션, 씬, 효과를 제작하기 좋다. | 전역 입력·트레이·설치·브랜딩 빌드를 데스크톱 유틸리티 수준으로 통합해야 한다. | 복잡한 캐릭터 세계·게임성이 제품의 중심이 될 때 유리하다. |
| Swift/AppKit + Windows 네이티브 UI | OS 동작과 입력 API에 직접 접근할 수 있다. | 두 UI·배포 구현을 유지해야 한다. 공유 Rust 코어를 붙여도 셸은 이중화된다. | OS별 경험을 최우선으로 할 때 가능하지만 초기 범위에는 비용이 크다. |

대안의 공식 근거: [Flutter 데스크톱](https://docs.flutter.dev/platform-integration/desktop), [Flutter 플랫폼 연동](https://docs.flutter.dev/platform-integration/platform-channels), [Qt 투명 위젯](https://doc.qt.io/qt-6/qwidget.html#creating-translucent-windows), [Qt 창 플래그](https://doc.qt.io/qt-6/qt.html#WindowType-enum), [Godot DisplayServer](https://docs.godotengine.org/en/latest/classes/class_displayserver.html), [AppKit NSWindow](https://developer.apple.com/documentation/appkit/nswindow), [WPF Window](https://learn.microsoft.com/en-us/dotnet/desktop/wpf/windows/). 이 자료가 제품의 전역 입력·배포 전체를 기본 제공한다는 의미는 아니다.

## 3. Electron과 Tauri 상세 비교

| 항목 | Electron | Tauri | 이 제품의 선택 기준 |
| --- | --- | --- | --- |
| 화면 엔진 | 포함된 Chromium 기준 | Windows WebView2 / macOS WKWebView | 단순 PNG·Canvas 렌더러로 엔진 차이의 영향을 줄인다. |
| 제품 코어 | Node/TypeScript 중심, 필요시 네이티브 모듈 | Rust 중심, 화면과 IPC로 통신 | 스키마·검증·입력·행동·빌드를 Rust에 모은다. |
| 일반 전역 입력 | 별도 모듈/네이티브 구현 필요 | 별도 모듈/네이티브 구현 필요 | 프레임워크가 입력 문제를 자동 해결한다고 평가하지 않는다. |
| 단축키 | globalShortcut | global-shortcut 플러그인 | 앱 제어용 조합키에만 사용한다. 일반 키 관찰은 별도다. |
| 클릭 통과 | setIgnoreMouseEvents | setIgnoreCursorEvents | 둘 다 전체 창 통과를 기준으로 구현·검증한다. |
| 투명 창 크기 | 공식 문서에 사용자 리사이즈 제약 명시 | 창 API를 제공하지만 실제 OS 조합 검증 필요 | 공통적으로 캐릭터 배율을 사용하고 창 경계는 Runtime이 관리한다. |
| 리소스 사용 | 실제 앱·프로세스 구성을 측정해야 한다. | 실제 WebView·합성 부하까지 측정해야 한다. | Tauri가 언제나 더 적은 메모리·전력을 쓴다고 단정하지 않는다. |
| 제작자 환경 | Node와 네이티브 모듈 빌드 환경 | Rust·플랫폼 도구체인, 웹 화면 빌드 환경 | Builder 진단 기능으로 누락된 환경을 설명한다. |
| 독립 앱 생성 | 메타데이터와 자산을 반영해 다시 패키징 | 메타데이터와 자산을 반영해 다시 빌드·패키징 | 이름만 바꾼 공용 실행기가 아니라 프로젝트별 앱을 만든다. |

근거: [Tauri WebView](https://v2.tauri.app/reference/webview-versions/), [Electron 네이티브 모듈](https://www.electronjs.org/docs/latest/tutorial/using-native-node-modules), [Electron 단축키](https://www.electronjs.org/docs/latest/api/global-shortcut), [Tauri 단축키](https://tauri.app/plugin/global-shortcut/), [Electron 클릭 통과](https://www.electronjs.org/docs/latest/tutorial/custom-window-interactions), [Tauri 창 API](https://v2.tauri.app/reference/javascript/api/namespacewindow/), [Electron 투명 창 제약](https://www.electronjs.org/docs/latest/tutorial/custom-window-styles#limitations).

## 4. 선택 근거

Tauri + Rust를 선택하면 제작 도구와 생성 앱에 필요한 프로젝트 파서, 검증기, 행동 엔진, 네이티브 입력 계층을 Rust로 공유할 수 있다. 화면은 얇게 유지하고, 생성된 앱에 Studio 편집기와 빌드 도구를 넣지 않을 수 있다. 이 모듈 구성의 적합성이 선택의 핵심이다.

Tauri가 시스템 WebView를 사용하는 점은 배포 크기 측면의 후보 이점이다. Windows WebView2 설치·다운로드 비용을 제외한 수치와 전체 설치 비용을 혼동하지 않는다. 작은 설치 파일이 낮은 실행 메모리나 배터리 사용량의 증거는 아니다.

Electron도 요구사항을 구현할 수 있다. 다만 이 프로젝트는 이미 Tauri + Rust로 결정했으므로 두 호스트를 동시에 구현하지 않는다. 관련 문제를 재현한 뒤에만 대안 전환을 검토한다.

## 5. 최신 문서 확인으로 달라지는 점

### macOS 투명 창

과거 자료의 `macOSPrivateApi: true` 필수 설명을 그대로 적용하지 않는다. 공식 **@tauri-apps/api 2.12.1 릴리스**는 macOS 투명·전체 화면에 해당 플래그가 더 이상 필요하지 않다고 밝히며, 현재 설정 문서도 관련 변경을 반영한다. 구현 시 이 변경이 포함된 코어·API·CLI 조합을 확인해 잠금 파일에 고정한다. 이 사실을 App Store 승인 보장으로 해석하지 않는다. [릴리스](https://v2.tauri.app/release/@tauri-apps/api/v2.12.1/), [설정](https://v2.tauri.app/reference/config/)

### 투명 창과 GPU

Tauri 저장소에는 macOS 투명 창의 합성 부하를 보고한 [이슈 #15471](https://github.com/tauri-apps/tauri/issues/15471)이 있다. 단일 환경의 보고이며 모든 버전·기기에서 같은 결과라는 뜻은 아니다. 이 제품은 창 면적·유휴 재그리기·고주사율 환경을 실제 측정하고, 화면 전체를 덮는 투명 창을 기본 설계로 사용하지 않는다.

## 6. 화면 기술과 입력 기술의 후속 결정

- **화면 제안**: TypeScript + Vite + Canvas 2D. 최초 PNG 시트 애니메이션에는 충분한 기능만 구현한다.
- **PixiJS**: 레이어·이펙트·많은 스프라이트가 필요해지면 검토한다. 처음부터 필수 의존성으로 만들지 않는다.
- **React/Svelte**: Studio UI 복잡도를 보고 결정한다. 캐릭터 프레임마다 UI 컴포넌트를 갱신하지 않는다.
- **Rust WASM UI**: 언어 통일 이점은 있으나 초기 빌드·디버깅 경로를 늘리므로 기본 제안에서는 제외한다.
- **입력**: Windows Raw Input과 macOS listen-only event tap을 우선 검증한다. 패키지를 도입해도 어댑터 계약·수명·권한·OS 지원을 직접 확인한다.

입력 근거: [Microsoft Raw Input](https://learn.microsoft.com/en-us/windows/win32/inputdev/about-raw-input), [RIDEV_INPUTSINK](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-rawinputdevice), [Apple passive tap](https://developer.apple.com/documentation/coregraphics/cgeventtapoptions), [Apple 엔지니어의 입력 모니터링 설명](https://developer.apple.com/forums/thread/724608).

## 7. 채택 이후 기술 검증

선택 자체는 확정됐다. 아래는 기능 구현의 통과 조건이다.

- 두 OS에서 일반 사용자 권한으로 입력을 관찰하고 원래 앱이 동일 입력을 받는다.
- 미서명/로컬 서명 배포본에서 첫 실행·입력 권한·재실행을 재현한다.
- 투명 창이 클릭을 통과하고 포커스를 빼앗지 않는다.
- 고DPI·혼합 배율·모니터 제거에서 위치가 유효하다.
- CPU·GPU·메모리와 시작 시간을 같은 조건으로 측정한다.
- 서로 다른 두 프로젝트에서 만든 앱이 독립적으로 설치·실행된다.

측정 전에는 프레임워크별 MB·배수·배터리 절감 수치를 제품 성능으로 제시하지 않는다.
