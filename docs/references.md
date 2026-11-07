# 검토 자료와 근거

확인 기준일: 2026-11-07. 변동 가능한 버전·API·OS 정책은 구현 시 다시 확인하고 실제 잠금 파일·시험 결과를 함께 남긴다. 아래는 설계에 사용한 1차 자료다.

## Tauri

| 자료 | 문서에서 사용하는 사실 |
| --- | --- |
| [Process Model](https://v2.tauri.app/concept/process-model/) | Rust 코어와 OS WebView 프로세스의 구분 |
| [WebView versions](https://v2.tauri.app/reference/webview-versions/) | Windows WebView2와 macOS WKWebView |
| [Window API](https://v2.tauri.app/reference/javascript/api/namespacewindow/) | 클릭 통과·포커스·위치·크기 API와 플랫폼 주의사항 |
| [Global Shortcut](https://tauri.app/plugin/global-shortcut/) | 전역 조합키 등록. 일반 입력 전체 관찰과 구분 |
| [Channels](https://v2.tauri.app/develop/calling-frontend/#channels) | 순서 있는 상태 스트리밍을 위한 IPC 선택 |
| [Capabilities](https://v2.tauri.app/security/capabilities/) | 창별 권한 범위 설계 |
| [CSP](https://v2.tauri.app/security/csp/) | 설정에 명시해야 활성화되는 CSP, 신뢰하는 로컬 자산으로 제한 |
| [Embedding Additional Files](https://v2.tauri.app/develop/resources/) | bundle resources와 OS별 resource 경로 해석 |
| [Configuration](https://v2.tauri.app/reference/config/) | 제품 메타데이터·창·번들 설정 |
| [API 2.12.1 release](https://v2.tauri.app/release/@tauri-apps/api/v2.12.1/) | macOS 투명·전체 화면의 예전 private-api 플래그 요구 변경 |
| [CLI](https://v2.tauri.app/reference/cli/) | 생성 템플릿 빌드·번들 도구 |
| [Prerequisites](https://tauri.app/start/prerequisites/) | OS별 제작자 도구체인 |
| [Windows installer](https://tauri.app/distribute/windows-installer/) | Windows 설치와 WebView2 준비 방식 |
| [macOS ad-hoc signing](https://v2.tauri.app/distribute/sign/macos/#ad-hoc-signing) | 인증된 발행자 identity 없이 가능한 로컬 서명 설정 |
| [Windows signing](https://v2.tauri.app/distribute/sign/windows/) | Windows 서명 연동 검토 자료 |

## OS 입력·실행 승인

| 자료 | 사용하는 사실 |
| --- | --- |
| [Microsoft Raw Input](https://learn.microsoft.com/en-us/windows/win32/inputdev/about-raw-input) | 백그라운드 입력 관찰 설계 |
| [RAWINPUTDEVICE](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-rawinputdevice) | INPUTSINK와 EXINPUTSINK 차이 |
| [WM_INPUT_DEVICE_CHANGE](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-input-device-change) | DEVNOTIFY 등록 후 장치 도착·제거 알림 |
| [GetAsyncKeyState](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getasynckeystate) | 현재 눌림 대조, 최근 입력 비트·보안 데스크톱·물리/논리 버튼의 한계 |
| [Windows 11 release information](https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information) | 25H2 지원 기준과 릴리스 시점 실제 빌드 기록 |
| [Apple CGEventTap](https://developer.apple.com/documentation/coregraphics/cgevent/tapcreate(tap:place:options:eventsofinterest:callback:userinfo:)) | tap 생성·run loop·실패 처리 |
| [Apple CGEventTapOptions](https://developer.apple.com/documentation/coregraphics/cgeventtapoptions) | passive listener와 active filter 차이 |
| [Apple DTS 답변](https://developer.apple.com/forums/thread/724608) | CGEventTap와 입력 모니터링 점검/요청 API |
| [CGEventSource keyState](https://developer.apple.com/documentation/coregraphics/cgeventsource/keystate(_:key:)) | 키 상태 대조 후보 API, 실제 지원 OS·입력 중단 상태에서 시험 |
| [Apple TN2150](https://developer.apple.com/library/archive/technotes/tn2150/_index.html) | 보안 입력이 이벤트 전달에 미치는 영향. 갱신 중단 보관 문서이므로 현재 OS 실증 필요 |
| [Apple 앱 실행 안내](https://support.apple.com/en-gb/102445) | 다운로드 앱의 OS 실행 승인 |
| [Microsoft SmartScreen](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation) | 미서명/셀프 서명 앱의 신뢰 경고와 정책 제약 |

## 한글 키워드 변환

[Unicode Core Specification §3.12](https://www.unicode.org/versions/Unicode18.0.0/core-spec/chapter-3/)의 현대 한글 음절 분해 알고리즘을 사용한다. 음절을 물리 키로 치환하는 표와 Shift·입력 모드 의미는 [제품 입력 계약](input-and-keywords.md)에 고정한다. Unicode 음절 분해 자체가 현재 IME 모드나 실제 입력 텍스트를 판독하는 기능을 뜻하지 않는다.

## 대안

| 자료 | 비교 범위 |
| --- | --- |
| [Electron Process Model](https://www.electronjs.org/docs/latest/tutorial/process-model) | Chromium/Node 기반 구조 |
| [Electron 투명 창](https://www.electronjs.org/docs/latest/tutorial/custom-window-styles) | 투명 창과 리사이즈 제약 |
| [Electron 창 상호작용](https://www.electronjs.org/docs/latest/tutorial/custom-window-interactions) | 클릭 통과 |
| [Electron globalShortcut](https://www.electronjs.org/docs/latest/api/global-shortcut) | 전역 조합키와 일반 입력의 구분 |
| [Electron 네이티브 모듈](https://www.electronjs.org/docs/latest/tutorial/using-native-node-modules) | 네이티브 코드와 버전/패키징 고려 |
| [Flutter desktop](https://docs.flutter.dev/platform-integration/desktop) | 데스크톱 배포와 플러그인 |
| [Flutter native channels](https://docs.flutter.dev/platform-integration/platform-channels) | 플랫폼 API 연동 경로 |
| [Qt QWidget](https://doc.qt.io/qt-6/qwidget.html#creating-translucent-windows) | 투명 위젯 |
| [Qt WindowType](https://doc.qt.io/qt-6/qt.html#WindowType-enum) | 창 플래그 |
| [Godot DisplayServer](https://docs.godotengine.org/en/latest/classes/class_displayserver.html) | 투명 창·마우스 통과 기능 |
| [Apple NSWindow](https://developer.apple.com/documentation/appkit/nswindow) | OS 네이티브 셸 대안 |
| [Microsoft WPF Window](https://learn.microsoft.com/en-us/dotnet/desktop/wpf/windows/) | Windows 네이티브 셸 대안 |

## 버그 보고의 취급

[Tauri #15471](https://github.com/tauri-apps/tauri/issues/15471)은 macOS 투명 창 합성 비용에 대한 재현 보고다. 이를 위험 탐지와 자체 측정 항목에 사용한다. 보고된 수치가 이 제품에서 측정됐거나 모든 Tauri 앱에 적용된다고 주장하지 않는다.

## 직접 검증하지 않은 사항

- 참고 [인스타그램 릴스](https://www.instagram.com/reels/DfCbXESznAc/)는 로그인 제한으로 세부 동작을 확인하지 못했다.
- 실제 OS별 입력·투명 창·설치·성능 검증은 아직 하지 않았다.
- 입력 API·직접 어댑터 방향, OS 하한, 최초 UI 구성은 ADR 0002/0003으로 결정했다. 실제 바인딩·Tauri/Rust/웹 도구 버전은 M0 실증과 잠금 파일로 기록해야 한다.
- v1 구조·JSON·명령 계약은 구현 기준이다. 성능 예산은 목표이며 실행 가능한 구현이나 이미 달성한 수치를 의미하지 않는다.
