# 2026-11-07 개발 검증 기록

상태: 구현 진행 중. **M0 전체·M3 MVP 출시 완료를 뜻하지 않는다.**

## 환경

- 로컬: Apple Silicon arm64, macOS 14.4.1 (23E224), Xcode 15.3. 정식 지원 하한인 macOS 15 미만이므로 개발 환경 증거만 제공한다.
- Rust/Cargo 1.99.1, Node v24.18.0, npm 11.16.0.
- Rust Tauri 2.12.5 / tauri-build 2.7.4, JS API·CLI 2.12.4, Vite 6.4.1, TypeScript 5.9.3. 실제 의존성은 Cargo.lock·package-lock.json에 기록한다.
- Windows 실기 및 다른 macOS 기기 접근 없음. 실제 다운로드·설치·권한·전력 시험은 미실시다.

## 확인한 증거

| 범위 | 결과 | 증거·한계 |
| --- | --- | --- |
| 프로젝트·PNG·경로·동작 검증 | 자동 테스트 통과 | buddy-project 17개: 키워드 변환 3개·번들 진입 파일 resolver 2개·통합 12개. 현대 한글 11,172개 음절 변환과 실제 진입 파일명 오류 위치를 포함 |
| 공통 행동 엔진·반응 계측 | 자동 테스트 통과 | buddy-runtime 29개. 기존 행동 21개와 지연/누락/ACK 상관·후보 분해·큐 대기·키워드 최종 키 회귀 8개. 실제 OS 입력 시험을 대체하지 않음 |
| 입력 큐 | 자동 테스트 통과 | buddy-platform 3개. overflow epoch·이동 병합 시 키 전이 보존·callback 시각 보존 |
| Player 설정·탐색 정책 | 자동 테스트 통과 | Player 10개. 미래/과대 파일 보존·초기화·백업 복구·백업 쓰기 실패, 편집 세션 ID, 혼합 DPI·모니터 복구·anchor 여백, 로컬 origin/entry 제한과 원격·파일·유사 도메인 거부 |
| Builder snapshot·팩·취소 | 자동 테스트 통과 | buddy-builder 16개. 원본/자산 변경·팩 변조 거부·프로세스 그룹 취소, 설정·캐시·고지·식별 격리, 고정 도구 진단, 명시적 진입 파일 snapshot/산출물 검사 |
| 전체 Rust | 75개 통과, clippy 경고 없음 | cargo test --workspace 및 cargo clippy --workspace --all-targets -- -D warnings |
| 웹·계약 생성 | 빌드 통과 | npm run build:player. Rust에서 JSON Schema·TypeScript DTO 생성, tsc·Vite 완료 |
| 렌더러 ACK | 자동 테스트 4개 통과 | 모의 Canvas/RAF로 동일 픽셀 새 instance·30 FPS 제한·이미지 대기 중 상태 교체·숨김·오류/복구를 확인. 실제 Canvas 픽셀·WebView UI·OS 성능은 미검증 |
| Windows 입력 소스 | 대상 컴파일 검사 통과 | buddy-platform·buddy-cli --tests --target x86_64-pc-windows-msvc. 키보드 크기 Raw Input packet 회귀 테스트도 컴파일 검사. 실제 Windows 실행·설치 증거 아님 |
| 로컬 native 입력 | 권한 없음 확인 | buddy input-probe: 권한 미허용, 감지 중지, 원문 저장 없음. 실제 키 감지 미검증 |
| 로컬 개발 Player | 컴파일·프로세스 시작 확인 | npm run player:dev. UI·포커스·클릭 통과가 통과했다는 증거는 아님 |
| 개발용 .app | 생성·서명 정적 검사 통과 | target/debug/bundle/macos/Typing Cat.app. codesign --verify --deep --strict, bundle ID·버전·PNG/JSON/라이선스 포함 확인. minimumSystemVersion=14.0인 개발 빌드 |
| UI 시각·조작 검증 | 첫 캐릭터 표시 확인, 나머지 미검증 | 사용자 실행·조작 승인 후 v11을 실행. 초기 연결 timeout 뒤 같은 실행 프로세스에 연결해 tauri://localhost의 Typing Cat canvas와 실제 캐릭터 화면 확인. 트레이·설정·편집·전역 입력 시험은 사용자가 직접 진행하기로 함 |
| 독립 Builder 전체 | 개발용 두 샘플 빌드 통과 | 같은 팩에서 Typing Cat·Click Mascot의 .app·DMG·안내·보고서를 생성. 서로 다른 identifier·실행 파일·아이콘·리소스, 제품별 DMG 1개, 해시와 ad-hoc 서명 정적 검사. 지원 OS 실행·외부 설치 증거 아님 |
| 원본 checkout 접근 차단 빌드 | 개발용 두 샘플·한글 진입 파일 앱 통과 | v10 CLI·팩·프로젝트를 별도 디렉토리로 복사. OS sandbox로 원래 저장소의 file-read를 거부하고 실제 읽기 실패 probe 후 세 .app/DMG 빌드·해시·서명·번들 프로젝트 검증. 지원 OS/외부 기기 시험은 아님 |
| CLI·미리보기 배포 묶음 | 개발용 생성·부분 검증 통과 | CLI·팩·공통 preview Player를 묶고 해시·서명 검사. 별도 임시 작업 디렉토리에서 new/validate/모의 키워드·Space 반응 통과. preview UI는 미실시 |
| 공통 종속성 고지 | 생성·복사·무결성 검사 통과 | Rust 283개·웹/빌드 도구 4개와 소스 아카이브 17개. CLI·팩·preview·두 앱 리소스 및 산출물 LICENSES의 고지 파일 hash 일치. 법률 심사 통과를 뜻하지 않음 |
| Windows 설치 identity | 구현·회귀 통과, 실제 컴파일/설치 미검증 | 전체 identifier SHA-256 기반 경로/registry, slug+짧은 hash 바로가기 및 충돌 보호, 같은 이름 MSI 추정 제거. Windows 대상 config 생성·이름/버전 변경 및 다른 앱 격리 테스트 |
| CI | workflow 작성 | macOS arm64·Windows native 테스트, 생성 계약 차이·포맷·고지 검사와 Windows 두 샘플 NSIS 빌드·CLI 배포 묶음·산출물 보관 구성. 아직 push·원격 실행하지 않았으며 Windows 11 실기 입력 검증을 대체하지 않음 |
| 누락된 Rust 자동 설치 금지 | 실제 격리 환경 검사 통과 | 빈 RUSTUP_HOME·잘못된 부모 toolchain/작업 폴더 override와 로컬 다운로드 관찰 서버로 doctor·cargo wrapper·build 실행. 종료 코드 1, 다운로드 요청 0, 설치 toolchain 0, snapshot/컴파일 시작 없음·실패 단계/로그 보존. artifacts/validation/toolchain-no-install-v9.json |
| 디스크 부족 시 기존 결과 보존 | 실제 임시 볼륨 검사 통과 | 96 MiB HFS+ 시험 볼륨에 이전 성공 산출물을 복사하고 여유를 512 KiB로 제한. verify-template 단계의 ENOSPC/종료 1, 이전 320개 파일·원본 프로젝트 불변, 새 성공 결과 없음 확인. 시험 볼륨 해제·삭제 완료. artifacts/validation/disk-full-v11.json |

키·문자 원문 이력은 진단에 남기지 않는다. 테스트는 통제된 모의 입력 fixture를 사용한다. 현재 개발 앱은 동일 컴퓨터 빌드이므로 B03의 외부 기기 독립 실행 완료를 주장하지 않는다.

## 남은 출시 gate

- M0: Windows와 지원 macOS 실기의 입력·권한·투명 창·포커스·DPI, 두 앱 최소 외부 설치.
- M1: 작성한 CI의 실제 원격 실행과 Windows 네이티브 runner 코어 테스트 결과.
- M2: Player·preview UI, 포커스·클릭 통과·트레이·편집 완료/취소·저장 실패 실증. 편집 세션 ID·모니터 선택/좌표/제거 복구·소스별 상태·숨김 렌더 중단·native rearm은 구현했으나 실제 다중 모니터·절전/잠금/보안 입력 검증이 필요하다.
- M3: 지원 OS·외부 제작/실행 기기의 두 앱 설치·설정·제거·업데이트 격리, 배포 CLI의 원본 checkout 없는 빌드와 preview 실행. Windows NSIS identity와 고지 생성/동봉·lock/hash 검사는 구현했지만 Windows에서 실제 컴파일·설치/업데이트/제거·격리 검증은 미실시다. 현재 보고서는 thirdPartyNotices.status=bundled-and-verified와 고지 수·hash를 기록한다. 작업별 로그도 보존·해시한다.
- N01~N04/N09: 지속적인 반응 ACK·native 큐 대기 포함 지연·제외/누락 집계·30 FPS fixture·진단 JSON 저장을 구현하고 자동 회귀를 통과했다. 실제 지원 OS의 Release 성능·전력·지연·시작 시간과 2개 앱 동시 실행은 미측정이다. 내부 main 진입→첫 정상 ACK는 외부 프로세스 콜드 스타트 전체 시간이 아니며 N04 통과로 인정하지 않는다.

## P0 완료 기준 재대조

아래 표의 로컬 증거는 전체 제품 완료 판정이 아니다. 지원 OS·외부 실행·실제 입력에 대한 증거가 없으면 해당 요구사항은 미완료로 둔다. 설정 파일 읽기 한도·저장 실패 회귀를 추가하고 현재 소스를 v11 팩과 대조했다. 자동 검사 당시 macOS 14.4.1의 CLI input-probe에서 inputPermission=needsRequest, keyboard/mouse capture=stopped, observedEvents=0을 확인했다. 입력 권한 요청이나 키 주입은 수행하지 않았다. 해당 CLI 결과는 artifacts/validation/current-input-status.json에 기록했으며 실행 중인 Typing Cat 앱의 현재 권한·수신 상태를 대신하지 않는다.

| 요구사항 | 확보한 증거 | 완료에 추가로 필요한 증거 |
| --- | --- | --- |
| B01 | JSON/경로/이미지/참조/미래 버전·실제 파일명 오류 자동 검사 | 지원 Windows 파일 시스템에서 실행한 검사 |
| B02 | 두 샘플과 한글 파일명 앱의 macOS 개발 패키징 | 지원 OS별 Release 두 앱 빌드 |
| B03 | 번들 데이터 검증·원본 접근 차단 빌드 | 개발 도구 없는 외부 기기의 설치·앱 실행 |
| B04 | macOS 개발 빌드·Windows 코어 대상 컴파일 검사 | Windows 네이티브 빌드와 지원 macOS 빌드 |
| B05 | snapshot 변경 거부·프로세스 그룹 취소·단계/로그/실패 보고·최종 확정, Mac 임시 볼륨 ENOSPC에서 기존 결과 보존 | Windows 취소·실패·디스크 부족 및 패키징 중 실패 검증 |
| B06 | 세 앱의 번들 데이터 hash·원본 프로젝트 이동 후 재검증 | 원본 없는 외부 기기의 앱 실행 |
| B07 | ad-hoc .app/DMG·해시·고지·설치 안내 | Windows 설치 파일과 두 OS 다운로드 설치 |
| B08 | ID·경로·설정 namespace·잠금·NSIS 분리 코드/회귀 | 두 앱 동시 실행·업데이트·제거 격리 |
| B09 | 배포 CLI·팩만 사용한 checkout 접근 차단 빌드·doctor 진단 | 지원 OS별 별도 제작 환경의 전체 경로 |
| R01 | 투명·무테·비포커스 창 설정 코드 | 두 OS의 실제 표시·포커스 관찰 |
| R02 | 배치/편집 세션·모니터 복구 좌표 자동 검사 | 드래그·완료/취소·재시작·실제 모니터 제거 |
| R03 | 배율 범위·anchor 여백·작업 영역 fit·DPR 처리 | 혼합 DPI·실제 확대/축소 시 잘림 없음 |
| R04 | 모드별 ignore-cursor-events·트레이 복구 코드 | 뒤 앱의 클릭·선택·스크롤과 편집 전환 |
| R05 | OS 키 어댑터·정규화·held/rearm·코어 회귀 | 다른 앱 전경의 실제 down/up·modifier·원래 입력 보존 |
| R06 | 마우스 어댑터·bounded queue·전이 보존 회귀 | 실제 버튼/휠·고폴링·전역 위치 수신 |
| R07 | 우선순위·쿨다운·중단 폐기·종료·이동 코어 검사 | 실제 렌더와 입력을 결합한 행동 관찰 |
| R08 | 트레이 항목·모드/소스 전환 코드 | 트레이 조작·숨김·설정·편집·종료 정리 |
| R09 | 토글/소스별 권한·수신 분리·무권한 중지 확인 | 실제 허용·거부·철회·명시적 재시도 |
| R10 | epoch/held/속도/sequence reset·fresh-input 회귀 | 잠금·절전·장치 제거·보안 입력·감지 재개 |
| R11 | 설정/백업/미래 버전·과대 파일 보존·백업 쓰기 실패 회귀 | UI 저장 실패·중간 종료·업데이트 설정 보존 |
| R12 | KPS 창·안정화·유휴·repeat 제외·reset 코어 검사 | 다른 앱 타이핑에 따른 실제 속도 애니메이션 |
| R13 | 현대 한글 전체 변환·Shift·KMP·timeout·충돌 회귀 | 실제 한/영 모드·전역 물리 입력 순서 반응 |
| N01 | 측정 절차 준비 | Release 프로세스 트리 메모리 실측 |
| N02 | 정적 유휴 중단·30 FPS 제한·시험팩 | CPU·GPU·전력 실측 |
| N03 | 큐 대기 포함 ACK 상관·분위수·제외/누락 회귀 | 30 FPS 실제 입력 trial의 p95 |
| N04 | main 진입→정상 ACK 내부 계측 | 외부 콜드 스타트 전체 시간 실측 |
| N05 | lock/도구/팩/protocol/hash 보고·고정 Rust·자동 설치 금지 | 지원 OS별 동일 입력 재빌드·보고서 대조 |
| N06 | 선언형 스키마·제한된 snapshot·고정 팩·프로젝트 명령 없음 | Windows에서 같은 거부 fixture 실행 |
| N07 | 원문 없는 상태/진단·모의 입력 회귀·프로브 집계만 출력 | 실제 앱 실행 중 저장/로그/네트워크 관찰 |
| N08 | 서명·WebView·권한·설치/업데이트/제거 안내 | 실제 다운로드의 OS 신뢰/실행 승인 흐름 |
| N09 | 두 제품 ID와 시험 절차 준비 | 두 앱 동시 실행 자원·시작·입력 지연 실측 |
| N10 | 창별 manifest·Rust 호출자 검사·CSP/asset scope·탐색 정책 회귀 | 실제 WebView의 금지 IPC·탐색·파일 접근 거부 |

M0~M3 종료를 위한 지원 OS 시험, 실제 원격 CI 실행, 외부 설치와 성능 측정은 미완료다. S01~S04의 Studio GUI는 합의한 P1/M4 범위이며 이 표의 P0를 대신하지 않는다.

### 재개에 필요한 외부 조건

지원 OS 실기 환경이 없는 동일한 제약을 여러 연속 작업에서 재확인했다. 현재 연결된 macOS 14.4.1은 지원 하한 미만이며 사용자는 현재 기기만 사용 가능하다고 확인했다. 앱 접근은 이후 승인됐고 첫 화면도 확인했다. 입력 감지의 현재 상태와 수동 시험 결과는 사용자 확인을 기다린다. Windows 및 지원 macOS에서의 실제 UI·설치·성능 결과를 로컬 모의 검사나 개발 패키징으로 대체하지 않는다.

다음 시험을 재개하려면 Windows 11 25H2 이상 x64와 macOS 15 이상 arm64의 시험 환경, Player 화면을 관찰·조작할 수 있는 앱 접근, 사용자가 설정하는 입력 모니터링 권한이 필요하다. 이를 기다리는 동안 지원 OS 기준을 낮추거나 미실시 항목을 통과로 변경하지 않는다. 시험 환경이 연결되면 위 P0 표의 미완료 항목부터 이어서 검증한다.

사용자는 현재 macOS 14 기기만 사용할 수 있다고 확인하고 v11 실행·화면 조작을 승인했다. 실제 첫 캐릭터 화면을 확인한 뒤 사용자가 테스트를 직접 진행하기로 했다. 자동 UI 조작을 중단했고 입력 모니터링 권한은 변경하지 않았다. 이후 전달받는 결과는 macOS 14 개발 환경의 부분 증거로 기록하며, 지원 OS의 Release·Windows·외부 설치 gate와 구분한다. OS 하한은 변경하지 않는다.

## 구현하면서 보완한 회귀

- 공유 Cargo 캐시의 bundle 폴더에 남은 이전 제품 DMG가 다른 앱 결과에 섞이는 문제를 제품 이름·버전 prefix 검사와 테스트로 수정했다.
- Tauri product 설정의 TOML 변환·기존 키 누락 문제를 실제 전체 빌드로 재현하고 회귀 테스트를 추가했다.
- Windows keyboard Raw Input packet이 union 전체보다 작은 경우의 잘못된 메모리 참조를 수정하고 Windows 대상 테스트를 컴파일 검사했다. Windows에서 실행한 테스트 결과는 아니다.
- 편집 명령마다 Host가 발급한 세션 ID를 검사하고 이전 세션 요청을 거부한다. 모드·소스 변경 중 오래된 revision의 입력 묶음은 처리하지 않는다.
- native 재개 시 현재 눌린 키·버튼은 동작 억제 상태로 전달한다. Shift의 실제 조합 상태는 새 키 step에 반영하고 기존 Control/Alt/Meta는 sequence를 초기화한다. 중지 중 해제된 키가 다음 첫 입력까지 막히는 문제와 이전 버튼 해제 반응을 테스트했다. 실제 전역 입력/OS 상태 변화는 별도 gate다.
- 반전 기준이 중앙이 아닌 anchor의 양쪽 범위 여백, 30 Hz snapshot 사이 편집 상태 변경 보존, 명시적인 숨김 렌더 중단을 반영했다. 시각 품질은 미검증이다. devicePixelRatio 변경 시 backing canvas를 갱신하고 편집 완료·설정 조회의 잠금 획득 순서도 정리했다.
- 같은 sprite 프레임으로 재시작한 새 action instance가 실제 그리기와 ACK에서 누락되지 않게 했다. ACK의 seq/epoch/instance를 발행 기록과 대조하고 오류·숨김·누락 표본을 성공 지연으로 만들지 않는다. 초기 숨김 WebView의 document.hidden 때문에 첫 준비 그리기가 막히지 않게 했으며 실제 시작 표시는 별도 UI gate다.
- 명시한 JSON 진입 파일을 snapshot·preview·생성 앱·산출물 검사까지 보존한다. 검사 단계의 기본 파일명 고정과 /var 같은 경로 별칭에서의 비교 오류를 회귀로 재현하고 공통 resolver와 canonical root로 수정했다. 잘못된 descriptor·변경된 자산은 거부한다.
- 두 창에 로컬 진입 문서 전용 navigation handler·새 창 Deny를 적용하고 base-uri/form-action CSP를 보완했다. URL 정책 자동 검사는 통과했지만 실제 WebView UI에서의 요청 차단 관찰은 미실시다.

## Windows 설치 컴파일 확인의 한계

macOS에서 NSIS 3.13 소스와 공식 toolset으로 별도 컴파일 검증을 시도했다. Homebrew 경로는 현재 CLT 환경에서 설치되지 않았고, 별도 Python venv/SCons로 만든 컴파일러는 Unicode true와 Exch $1만 있는 최소 예제에서도 std::bad_alloc으로 종료했다. 앱 템플릿을 넣기 전의 기본 변수 변환에서 재현돼 Windows 설치 코드 통과 증거로 사용하지 않는다. 실제 Windows NSIS 컴파일은 작성한 CI job과 지원 Windows 환경에서 확인해야 한다. 이 테스트용 파일은 실제 Player 또는 설치 가능한 Windows 샘플로 배포하지 않는다.

## 최신 산출물과 한계

최신 v11은 설정 파일 읽기 한도·과대 파일 보존과 저장 실패 회귀를 포함한다. CLI/preview 해시·서명과 팩 255개 파일의 현재 소스 일치를 확인했다. 원래 checkout 읽기를 차단한 `/tmp/buddy-source-isolation-v11-220aca37`에서 두 앱의 빌드 10단계가 모두 passed였으며 복사 후 각각 319개 파일 hash·서명·고지를 다시 검사했다.

| v11 산출물 | 경로 |
| --- | --- |
| CLI·공통 preview Player·팩 | `artifacts/cli/macos-arm64-0.1.0-dev-v11` |
| 타이핑 고양이 | `artifacts/current/typing-cat-0.1.0-9d785a3f-cb5f-4ed2-b174-ee6ec47f111e` |
| 클릭 마스코트 | `artifacts/current/click-mascot-0.1.0-f4eca23c-a7b0-4bff-8236-2d753d49ee9b` |
| checkout 접근 차단 기록 | `artifacts/validation/v11-source-isolation.json` |
| 복사 후 정적 검증 요약 | `artifacts/validation/v11-static-verification.json` |

v11 템플릿 hash: `7c884c366a52307ea27845c797427265bf1659fe90a397ec2c776f2b4756238c`. 두 결과는 macOS 14의 --development 산출물이며 지원 OS Release 성능과 앱 실행 증거는 아니다.

이전 v10은 고정 도구체인·진입 파일 보존·탐색 제한을 포함한다. CLI/preview 해시·서명과 팩 255개 파일의 당시 소스 일치를 확인했다. 원래 checkout 읽기를 차단한 `/tmp/buddy-source-isolation-v10-4a800c95`에서 세 앱의 빌드 10단계가 모두 passed였으며 결과를 아래 경로로 복사해 각 319개 파일 hash·서명·고지를 다시 검사했다. 한글 진입 파일 앱은 원본 프로젝트 폴더를 이동한 뒤에도 번들 데이터의 재검증 hash가 일치했다. 앱 프로세스를 실행한 증거는 아니다.

| v10 산출물 | 경로 |
| --- | --- |
| CLI·공통 preview Player·팩 | `artifacts/cli/macos-arm64-0.1.0-dev-v10` |
| 타이핑 고양이 | `artifacts/current/typing-cat-0.1.0-86fe5dc1-a58b-4ee8-9c8c-9bbf46733602` |
| 클릭 마스코트 | `artifacts/current/click-mascot-0.1.0-6a8d8382-6e7c-4a8c-8a09-4212cdbf4ea3` |
| 한글 진입 파일·앱 이름 시험 | `artifacts/current/custom-entry-buddy-0.1.0-093b2c33-8de7-415a-9046-c447cfa0b069` |
| checkout 접근 차단 기록 | `artifacts/validation/v10-source-isolation.json` |
| 복사 후 정적 검증 요약 | `artifacts/validation/v10-static-verification.json` |

v10 템플릿 hash: `3e98397142057d4e6bd909fbd1b498e8f29856794c60ab55a6988e536260151b`. 원본 접근 차단은 sourceReadProbeExit=1로 확인했고, 빌드 보고서 toolchainPolicy의 Rust=1.99.1·ready=true·automaticInstallation=false도 대조했다. macOS 14의 --development 결과이며 외부 기기 실행·지원 OS·실제 권한·성능 gate는 여전히 미실시다.

아래 v8은 성능 계측 추가 당시의 개발 산출물이다. CLI·preview의 SHA256SUMS와 서명, 팩 251개 파일과 당시 소스의 일치, 세 앱 각각 318개 파일 hash·서명·고지를 검사했다. 각 빌드 보고서의 9단계가 passed이며 실제 UI·OS 입력·성능은 not-performed다.

| v8 산출물 | 경로 |
| --- | --- |
| CLI·공통 preview Player·팩 | `artifacts/cli/macos-arm64-0.1.0-dev-v8` |
| 타이핑 고양이 | `artifacts/current/typing-cat-0.1.0-299e8353-fc91-404d-9f8b-80623384a22d` |
| 클릭 마스코트 | `artifacts/current/click-mascot-0.1.0-e60aa70b-b9b5-4510-8c48-04053144e339` |
| 30 FPS 시험 앱 | `artifacts/current/performance-30fps-0.1.0-50682d1b-70cc-4047-8484-517c8a5db034` |
| 정적 검증 요약 | `artifacts/validation/v8-static-verification.json` |

v8 템플릿 hash는 `3046773879109dc04d9c98f7ee092742f351997230cc23e3475649864bfcb40f`다. 세 앱은 서로 다른 identifier를 가지며 각각 해당 제품 DMG 1개만 포함한다. --development와 macOS 14 환경에서 만든 결과이므로 지원 OS 출시 gate를 통과한 것으로 표시하지 않는다. 계측기의 수치 자체는 실제 앱에서 아직 수집하지 않았다.

아래는 성능 계측 추가 전 v7의 패키징 검증이다. 최신 코드의 자동 검증과 구분한다. 각 경로는 gitignore 대상이며 같은 작업 디렉토리에서 확인할 수 있다.

| 산출물 | 경로 |
| --- | --- |
| CLI·공통 preview Player·팩 | `artifacts/cli/macos-arm64-0.1.0-dev-v7` |
| 타이핑 고양이 | `artifacts/current/typing-cat-0.1.0-43fd0ae8-a6da-4ddf-91ac-933abaf58a19` |
| 클릭 마스코트 | `artifacts/current/click-mascot-0.1.0-6181f24b-03c1-4033-9efc-09a5793adecc` |

v7 템플릿 hash: `0f9bb2fb0a24d48b1f019ee67c9bbb22372871a59270214bd90128b45679dcc3`. 당시 소스·팩의 모든 파일 hash가 CLI에 고정된 manifest와 일치했다. 두 앱 각각 `.app`와 해당 제품 DMG 1개만 포함하며, 안내·라이선스·최종 보고서·build.log의 SHA256SUMS와 app 서명을 검사했다. preview와 두 앱 내부의 고지가 외부 LICENSES 사본 및 고정된 manifest와 일치한다. 9단계가 모두 passed인 보고서를 생성했고, dataIdentity/dataKey도 개발용 설정 namespace와 대조했다. 이 검사는 실제 앱 실행 검증이 아니다.

이 기록의 산출물은 macOS 14에서 --development로 만든 개발용 결과다. 디버그 Player와 CLI preview의 설정 identity는 io.desktopbuddy.preview.<projectId>로 분리하며, 정식 생성 앱은 제품 identifier를 사용한다. 보고서의 dataIdentity/dataKey는 이 차이를 반영한다.

별도 작업 디렉토리에서 CLI 묶음만 복사해 new/validate/모의 입력을 실행했다. 여기에 더해 `/tmp/buddy-source-isolation-v7`의 별도 CLI·팩·두 프로젝트로 빌드하면서 OS sandbox가 원래 저장소 경로의 file-read를 거부하도록 했다. 원래 README 읽기가 권한 오류로 끝나는 probe를 먼저 확인했고, 그 상태의 두 앱 빌드·각 SHA256SUMS·서명 검사가 통과했다. 결과는 해당 디렉토리의 source-isolation-result.json과 작업 로그에 있다. 원본 접근이 없는 제작 경로의 개발 환경 증거이며, 같은 컴퓨터의 macOS 14와 개발 도구를 사용했으므로 지원 OS·외부 제작 기기 gate 전체를 통과한 것으로 간주하지 않는다.

실제 CLI에서 같은 identifier를 다른 projectId가 사용하는 빌드도 PRODUCT_IDENTITY_CONFLICT로 거부했으며 이전 성공 보고서들의 hash가 유지됐다. 개발용 패키지를 사용자 설치·공증·실제 권한·성능 통과로 표시하지 않는다.

## 재현 명령

```sh
npm ci
npm run build:player
npm run test:renderer
node scripts/cargo.mjs test --workspace
node scripts/cargo.mjs clippy --workspace --all-targets -- -D warnings
node scripts/cargo.mjs check -p buddy-player --features preview
node scripts/cargo.mjs check -p buddy-platform -p buddy-cli --tests --target x86_64-pc-windows-msvc
node scripts/cargo.mjs run --locked -p buddy-cli -- validate examples/typing-cat --json
node scripts/cargo.mjs run --locked -p buddy-cli -- validate examples/performance-30fps --json
node scripts/cargo.mjs run --locked -p buddy-cli -- input-probe --seconds 1
```

마지막 input-probe는 권한이 없으면 실패 종료하고 감지가 중지된 상태를 보고한다. 권한 요청은 명시적인 --request-permission 또는 Player 설정 동작으로만 수행한다. 도구·OS 버전이 달라지면 해당 조건과 결과를 갱신하고 미실시 항목을 통과로 바꾸지 않는다.
