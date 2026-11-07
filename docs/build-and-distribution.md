# 독립 앱 빌드와 배포 설계

상태: v1 구현 기준, 빌드·배포 실증 전 / 기준일: 2026-11-07

## 1. 배포 정책

사용자 결정에 따라 **공식 인증서 구매·공식 신뢰 서명·Apple 공증을 필수 조건으로 두지 않는다. 필요한 셀프 서명은 허용한다.**

| 대상 | 기본 정책 | 선택 가능 |
| --- | --- | --- |
| Windows | Authenticode 공식 서명 없이 사용자 단위 설치 파일 생성 | 로컬에 준비된 셀프 서명 identity 사용 |
| macOS | ad-hoc 서명으로 앱과 DMG 생성, 공증 없음 | 검증된 로컬 셀프 서명 identity 사용 |

ad-hoc은 인증서로 발행자 신원을 증명하는 셀프 서명과 다른 로컬 서명 형태다. macOS 기본값은 Tauri의 `bundle.macOS.signingIdentity`에 `-`를 적용하는 방식이다. 해당 방식은 공식 문서에 안내되어 있으며, OS의 사용자 실행 승인을 없애지는 않는다. [Tauri ad-hoc 문서](https://v2.tauri.app/distribute/sign/macos/#ad-hoc-signing)

Windows 셀프 서명도 공인 발행자 신뢰나 SmartScreen 평판을 자동으로 얻지 않는다. OS·조직 정책에 따라 계속 실행 선택이 제한될 수 있다. [Microsoft 설명](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)

인증서를 수신자의 신뢰 루트에 자동 등록하는 설치 동작은 만들지 않는다. 유료 인증서·계정·공증 자격증명은 기본 빌드에 필요하지 않다. 셀프 서명 키의 생성·보관·identity 선택은 제작자 로컬 환경에서 별도로 관리한다.

## 2. 세 가지 산출물

| 산출물 | 용도 | 수신자 개발 환경 필요 여부 |
| --- | --- | --- |
| 프로젝트 폴더/아카이브 | 다른 제작자가 열고 수정·재빌드 | 빌드하려면 필요 |
| 생성된 소스 프로젝트 | 개발자가 Runtime을 확장·검토 | 개발 시 필요 |
| **독립 앱/설치 파일** | 최종 사용자가 설치·실행 | **Rust·Node·Studio 불필요** |

최초 요구를 충족하는 결과는 세 번째다. 프로젝트 ZIP이나 공용 Player에 불러오는 데이터만 내보내고 독립 앱 빌드가 끝났다고 표시하지 않는다.

## 3. 빌드 파이프라인

```text
프로젝트 진입 파일 읽기
 → 경로·크기·참조 파일 사전 검사
 → 대상 OS/아키텍처·도구체인 진단
 → 허용된 프로젝트 파일과 자산의 불변 snapshot 생성
 → snapshot의 스키마·자산·식별자 최종 검증 / 키워드 steps 변환
 → snapshot·정규화 데이터·자산 hash 확정
 → 버전 고정된 Player 템플릿으로 작업 디렉토리 생성
 → 제품 이름·아이콘·identifier·버전·기본값·리소스 반영
 → 웹 렌더러 및 Rust 실행 파일 빌드
 → 선택한 로컬 서명 / 설치 파일 패키징
 → 산출물 정적 검사·해시·보고서 생성
```

생성된 소스만 바꾸고 런타임을 공유하는 방식이 아니라, 공통 소스에서 앱별 독립 실행 파일과 패키지를 만든다. 설치 후 원본 프로젝트 폴더가 없어도 실행된다.

템플릿과 종속성은 Builder/Studio가 공유하는 Engine 릴리스의 검증된 버전으로 고정한다. 사용자가 가져온 프로젝트의 빌드 스크립트·의존성·셸 명령은 실행하지 않는다. Tauri CLI의 config 기능을 쓰더라도 Builder가 허용 필드만 생성한다. [Tauri CLI](https://v2.tauri.app/reference/cli/)

### Builder와 템플릿 팩 배포

Studio보다 먼저 Windows x64/macOS arm64용 `buddy` CLI와 `player-template-pack`을 같은 릴리스로 배포한다. 팩에는 Player·buddy-project/runtime/platform 소스, 공통 renderer 웹 소스, Cargo/웹 잠금 파일, Rust toolchain 정의, 패키지 매니저 버전, 스키마·샘플·아이콘 생성 규칙·라이선스를 포함한다. pack manifest에 Builder/Runtime/템플릿/protocol 버전과 모든 파일 hash를 기록한다. CLI는 팩 무결성과 호환성을 검사하고 불일치하면 실패한다.

작업 디렉토리에 독립 Cargo·웹 workspace를 만들고 필요한 공통 소스를 함께 복사한다. source path 의존성은 그 디렉토리 안에서만 해석한다. 원래 개발 저장소·제작자 절대 경로·사용자가 제공한 Cargo/웹 설정을 참조하지 않는다. 미리 설치된 도구체인으로 잠금 모드 빌드하며 첫 공식 의존성 다운로드는 허용한다. Node·Rust·OS 빌드 도구의 자동 설치는 하지 않고 doctor가 준비 방법을 안내한다. CLI 릴리스 자체의 생성·서명·설치도 OS별 CI에서 검증한다.

doctor와 실제 빌드는 Engine의 rust-toolchain.toml에 고정된 Rust를 사용한다. 실행 폴더의 toolchain 파일이나 부모 RUSTUP_TOOLCHAIN으로 다른 버전을 선택하지 않는다. 자식 프로세스에도 RUSTUP_TOOLCHAIN과 RUSTUP_AUTO_INSTALL=0을 지정한다. 이 환경 변수의 동작은 [Rustup 공식 문서](https://rust-lang.github.io/rustup/environment-variables.html)를 따른다. doctor의 toolchainPolicy는 Rust/Cargo 정확한 버전, Node 22 이상·npm 실행 가능 여부, 대상 Rust 표준 라이브러리를 검사한다. 누락 시 준비 명령과 ready=false를 반환하고 종료 코드는 1이다. OS 하한은 host.meetsSupportBaseline으로 별도 표시하며, SDK·C++ 빌드 도구의 실제 작동은 해당 플랫폼 빌드 결과로 확인한다.

Builder는 diagnose-toolchain 단계를 snapshot 전에 수행한다. 도구가 준비되지 않았으면 의존성 설치·snapshot·컴파일을 시작하지 않고 실패 단계·오류·로그를 남긴다. Node 기반 개발·Player·CLI 패키징 스크립트도 같은 고정 버전과 자동 설치 금지 환경을 사용한다. 제작자 정의 CARGO_HOME의 bin을 사용하며 사용자의 전역 rustup 설정은 바꾸지 않는다.

Studio는 같은 CLI 릴리스의 Builder 라이브러리·팩을 동봉한다. Studio가 없어도 CLI가 동작하고, 두 제품의 템플릿 버전을 조용히 섞지 않는다. 지원 버전 숫자는 추정해서 문서에 쓰지 않고 M0에서 호환 조합을 실제 잠금 파일과 pack manifest로 확정한다.

### 빌드 작업 계약

- 작업 ID마다 별도 디렉토리와 로그를 사용하고 초기에는 빌드를 1개씩 처리한다. 출력 루트에 `build-<jobId>.log`를 남기고 성공 묶음에는 패키징/검사 시점까지의 `build.log` 사본과 hash를 포함한다. 실패·취소 보고서도 작업 로그를 가리킨다. 입력 이력을 로그로 수집하지 않는다.
- 취소는 프로세스 트리·자식 작업까지 정리하고 성공 상태를 남기지 않는다.
- 사전 검사에서 발견한 JSON·참조 자산·라이선스만 snapshot으로 복사한다. 원본 디렉토리를 통째로 복사하지 않으며 정규화한 canonical root·symlink·크기 검사를 복사에도 적용한다.
- 복사 전후 파일 identity·크기·수정 정보와 내용 hash를 비교하고 진입 JSON·manifest도 다시 확인한다. 변경·불안정 파일은 `PROJECT_CHANGED_DURING_SNAPSHOT`으로 실패하고 사용자가 재시도한다. OS 전체 디렉토리의 순간적 atomic snapshot을 보장한다고 표현하지 않는다.
- 복사된 파일만 최종 검증하고 그 후에는 원본을 다시 읽지 않는다. 빌드 보고서는 실제 사용한 snapshot과 정규화 데이터의 hash를 기록한다.
- 명시한 프로젝트 진입 파일명도 snapshot에서 보존한다. 미리보기에는 그 경로를 전달하고 생성 앱에는 프로젝트 폴더 밖의 project-entry.json descriptor를 넣는다. Player는 1 KiB 제한과 단일 파일명·프로젝트 경로 검사를 적용한다. descriptor가 잘못됐을 때 다른 프로젝트로 조용히 전환하지 않는다. 기본 진입 파일은 buddy.project.json이다.
- 경로와 이름은 구조화된 인자로 전달한다. `sh -c` 문자열에 삽입하지 않는다.
- 임시 생성물만 정리한다. 제작자의 프로젝트·이전 성공 산출물을 삭제하지 않는다.
- 결과 파일은 검사 완료 후 같은 출력 볼륨의 임시 디렉토리에서 최종 경로로 원자적으로 확정한다. 같은 버전의 기존 성공 산출물은 기본 덮어쓰기하지 않고 별도 작업 경로를 쓴다.
- 실패 메시지는 단계, 오류 코드, 대상, 해결 방향을 포함하고 개인 키·토큰은 출력하지 않는다.

## 4. 앱 식별과 브랜딩

앱별 productName, executable slug, bundle identifier, 버전, 아이콘을 빌드 전에 반영한다. 파일 이름만 바꿔서 별도 앱으로 간주하지 않는다.

identifier는 앱 데이터 디렉토리·중복 실행 잠금·패키지 식별에 일관되게 사용한다. Windows 설치/업그레이드 식별자도 제품 identifier에서 안정적으로 파생하며 새 제품은 다른 값을 사용한다. 같은 제품의 새 버전은 같은 식별자를 유지한다.

플랫폼 매핑의 기준 키는 identifier UTF-8의 SHA-256 전체 hex다. Windows 사용자 설치 경로·제거 registry key·단일 인스턴스 이름을 이 키로 분리하고 NSIS의 표시 이름 기본값에 맡기지 않는다. macOS CFBundleIdentifier와 앱 데이터 이름은 identifier를 사용한다. product.name은 표시용이라 변경해도 설치·데이터 ID가 바뀌지 않는다. slug는 동일 제품에서 유지한다. `platformIdentity` 매핑을 보고서에 남기고 업데이트·제거 fixture로 검증한다. Windows 바로가기는 유지되는 slug와 식별 hash 앞 8자리로 이름을 구분하고, 예상한 경로에 다른 앱의 바로가기가 있으면 덮어쓰지 않는다. 설치 경로·registry는 앞 8자리가 아닌 전체 hash를 사용한다. 동일 표시 이름의 macOS 앱 복사 경로 충돌은 설치 안내에서 구분하고 다른 제품을 대체하지 않는다.

사용자 데이터 경로는 제품 이름 변경과 분리한다. A와 B 앱의 트레이 이름·설정·제거 항목이 서로 혼동되지 않아야 한다. Studio의 미리보기 identifier는 실제 배포 앱과 별도다.

## 5. 플랫폼별 빌드 환경

| 호스트 | 최초 생성 대상 | 제작자 준비 사항 |
| --- | --- | --- |
| Windows 11 25H2 이상 x64 | Windows x64, NSIS 사용자 단위 설치 파일 | Rust, Microsoft C++ Build Tools, WebView2, 고정된 Node/웹 빌드 환경 |
| macOS 15 이상 Apple Silicon | macOS arm64 `.app`와 DMG | Rust, Xcode Command Line Tools, 고정된 Node/웹 빌드 환경 |
| macOS x64 | 후속 검증 후 macOS x64 | 해당 Rust target과 도구체인, 실제 Intel 실행 검증 |

Tauri 개발 환경 근거: [공식 prerequisites](https://tauri.app/start/prerequisites/). Node와 패키지 매니저는 제작 단계에서 사용하며 Player 실행 의존성으로 넣지 않는다.

로컬의 다른 OS용 빌드 요청은 즉시 지원 여부를 검사한다. 프로젝트를 대상 OS로 옮기는 경로와, 사용자가 연결한 GitHub Actions의 Windows·macOS runner에 빌드를 맡기는 경로를 제공한다. 원격 업로드는 저장소와 공개 여부를 표시한 Studio의 명시적인 빌드 동작으로 시작한다.

구현된 로컬 Studio는 저장된 revision의 검증된 참조 파일만 내보낸다. 같은 앱 identifier·projectId·content hash를 보존한 묶음은 다른 OS의 Builder에서 그대로 사용할 수 있다. `scripts/build-project.mjs`가 현재 호스트용 빌드를 수행한다. 수동 `build-project.yml`과 Studio 연동 `studio-build.yml`이 macOS arm64·Windows x64를 각각 빌드한다. hosted runner 결과는 개발용으로 표시한다. 연동 경계·인증·업로드·무결성·중단 복구는 [Actions 연동 문서](studio-actions.md)에 둔다.

## 6. 설치와 첫 실행

### Windows

기본은 관리자 권한이 필요 없는 사용자 단위 설치다. 설치 과정 자체의 UAC 여부와 앱 신뢰 경고를 구분한다. 다운로드한 미서명 앱에 SmartScreen 경고가 뜨는 경우 OS가 허용하는 사용자 승인 절차를 안내한다. Smart App Control이나 조직 정책이 실행을 차단하는 환경에 보편적인 우회가 가능하다고 약속하지 않는다. [Microsoft 근거](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)

Windows Runtime은 일반 사용자 데스크톱의 입력 관찰을 우선 지원한다. UAC 보안 데스크톱·잠금 화면·보호된 앱의 입력 감지는 범위 밖이다. 관리자 권한 실행을 기본 해결책으로 요구하지 않는다.

Windows WebView2가 없는 환경도 검사한다. 최초 설치는 공식 bootstrapper를 통한 준비 경로를 우선하고 네트워크 필요 여부를 표시한다. 오프라인 설치가 필요하면 별도 Runtime 포함/사전 설치 옵션을 검증하며 그 크기를 기본 설치 파일 크기와 구분한다. [Tauri Windows installer](https://tauri.app/distribute/windows-installer/)

### macOS

1. DMG를 열어 앱을 정해진 경로에 복사한다. 권한 승인 이후 앱 위치를 자주 바꾸지 않도록 설치 위치를 먼저 정한다.
2. 다운로드 앱 실행이 차단되면 사용자가 OS 설정에서 해당 앱의 실행을 승인하는 흐름을 안내한다.
3. 앱이 실제 실행된 뒤 입력 감지 목적과 범위를 설명하고, 사용자가 입력 모니터링 권한을 허용한다.
4. 재점검 또는 필요한 경우 앱 재시작으로 입력 어댑터를 다시 시작한다.

앱별 실행 예외 승인 절차는 [Apple 안내](https://support.apple.com/en-gb/102445), 입력 모니터링은 [Apple 엔지니어 설명](https://developer.apple.com/forums/thread/724608)을 기준으로 한다. OS 버전·관리 정책에 따라 승인 UI와 가능 여부가 다르므로 실제 배포본에서 검증한다.

listen-only 감지에 필요한 권한만 요청한다. 선택한 구현이 추가 손쉬운 사용 권한을 요구한다면 그 이유와 검사 결과를 기록하고, 필요하지 않으면 요청하지 않는다. 입력을 인식하는 것만으로 화면 녹화·카메라·마이크 권한을 요구하지 않는다.

키워드는 별도 텍스트 읽기가 아니라 같은 전역 키 순서 매칭이다. 추가 브라우저 확장·텍스트 읽기 권한·IME 교체를 요구하지 않는다. 설치 안내에는 캐릭터 앱에 포커스가 없어도 다른 작업 중 키·속도·키워드에 반응하며, 한글 키워드는 설정된 두벌식 패턴이고 실제 한/영 모드와 독립적이라는 점을 설명한다.

설치기는 macOS 입력 모니터링을 대신 허용할 수 없다. 셀프/ad-hoc 서명도 권한을 자동 승인하지 않는다. 업데이트·재빌드 후 권한 상태가 바뀔 수 있으므로 앱별 복구 흐름을 검증한다.

## 7. 서명과 무결성 검사

macOS ad-hoc 서명은 자산·아이콘·메타데이터를 최종 반영한 뒤 수행한다. 서명 후 번들 내용을 편집하는 배포 방식은 사용하지 않는다. 서명 형식·번들 구조를 확인하고, 실제 다운로드·다른 컴퓨터 실행은 별도 검증한다.

Windows 기본 산출물은 미서명이다. 선택한 셀프 서명 모드에서 identity가 없으면 조용히 미서명으로 전환하지 않고 실패를 반환한다. 셀프 서명은 서명 자체의 존재와 로컬 검증 결과를 기록하되 공인 신뢰로 표시하지 않는다.

OS 보안 기능을 전역 비활성화하거나, 설치기가 승인 절차를 자동 처리하는 방식은 제품에 포함하지 않는다.

## 8. 빌드 보고서

`build-report.json`에 다음 항목을 남긴다.

- 프로젝트 ID, 제품 identifier/버전, 프로젝트·자산 content hash
- Builder/Runtime/템플릿 버전과 잠금 파일 hash
- 템플릿 팩 manifest/hash, 정규화 스키마·protocol 버전, platformIdentity 매핑
- 호스트 OS, 대상 OS·아키텍처, 실제 도구체인 버전
- 단계별 시작·종료·결과, 캐시 사용 여부
- 산출물 상대 경로·바이트 크기·SHA-256
- 서명 모드 `none`/`ad-hoc`/`self-signed`, 공증 여부 false
- 정적 검사와 해당 컴퓨터 smoke test 결과
- **외부 기기 설치·입력 권한 검증 여부**: 미실시라면 미실시로 기록

공유 의존성 캐시는 템플릿 hash·target으로 분리하며, 템플릿 hash가 고정 도구체인 파일과 의존성 잠금 파일을 포함하고 Cargo 자체 fingerprint도 적용한다. 프로젝트·자산은 작업별 snapshot hash로 추적한다. 완성 앱 산출물 캐시는 사용하지 않고 매번 패키징·정적 검사한다. 이후 완성 앱 캐시를 도입한다면 키에 템플릿·도구체인·target·의존성·프로젝트·자산 hash를 모두 포함한다. 키와 인증서는 프로젝트 아카이브·로그·보고서에 넣지 않는다. 첫 다운로드/첫 컴파일 후 캐시가 있으면 완전 오프라인 재빌드가 가능한지는 별도로 측정한다.

SHA256SUMS는 앱·설치 파일과 안내·리소스 라이선스·최종 보고서를 포함한다. 보고서 자체의 artifacts 목록은 자기 참조 hash를 피하기 위해 보고서와 SHA256SUMS를 제외한다.

기본 산출물 묶음에는 설치 파일, build-report.json, SHA256SUMS, 설치·권한·업데이트·제거·지원 제한 안내, 포함 리소스와 공통 종속성의 라이선스 고지를 넣는다. 안내는 target별 고정 템플릿에서 생성하며 빌드 성공/실기 검증 여부·WebView 네트워크 필요성을 구분한다. 보고서에는 실제 입력·등록 keyword 원문을 나열하지 않고 프로젝트 hash로 식별한다.

## 9. 업데이트

최초 버전은 수동 업데이트다. 제작자가 같은 identifier와 높은 제품 버전으로 다시 빌드해 배포한다. 사용자 위치·배율은 보존하고, 앱 실행 승인·입력 권한이 유지되는지는 별도 확인한다. 자동 업데이트 서명 키는 OS 코드 서명과 다른 체계이므로 추후 ADR로 도입한다.

사용자가 명시한 설정은 새 기본값과 같았더라도 override를 보존한다. preferences 마이그레이션·손상 백업·미래 버전 처리 계약은 [아키텍처](architecture.md)를 따른다. downgrade는 공식 지원하지 않으며 새 버전 데이터가 읽히지 않을 때 원본을 덮어쓰지 않는다. 제거는 다른 앱 데이터에 영향을 주지 않고 해당 앱 사용자 설정은 기본 보존한다. 완전 초기화는 사용자에게 제공하는 별도 명시 동작이다.

## 10. 현재 개발용 CLI 묶음

`npm run package:cli -- --development --output <새 디렉토리>`는 CLI 실행 파일, `player-template-pack`, `player-preview`, 해시 목록과 개발 보고서를 함께 만든다. 기존 성공 묶음은 덮어쓰지 않는다. `buddy preview`는 검증한 snapshot을 별도 preview feature의 공통 Player에 전달하고 종료·Ctrl+C 시 임시 파일을 정리한다. 실제 배포 앱은 이 feature를 켜지 않으며 임의 외부 프로젝트 경로를 받지 않는다.

현재 산출물의 실제 검증 범위와 Windows 설치/업데이트 실기·외부 환경 gate는 [개발 검증 기록](validation/2026-11-07-development.md)을 따른다. 생성 성공만으로 완성된 출시 묶음으로 표시하지 않는다.

## 11. 고정된 설치 템플릿과 종속성 고지

Windows 설치기는 [Tauri CLI 2.12.4 NSIS 템플릿](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi)을 고정하고, `packaging/windows/upstream.json`에 원본·정규화 원본 hash와 수정 범위를 기록한다. 사용자 단위 경로와 설치/제거 키는 전체 identifier hash로 파생한다. 같은 표시 이름으로 MSI를 추정하는 경로는 사용하지 않는다. 사용자 지정 설치 폴더는 비어 있거나 같은 identity marker가 있어야 하며 다른 앱의 설치 폴더를 덮어쓰지 않는다. 실제 NSIS 컴파일·설치/업데이트/제거는 Windows 환경의 별도 통과 조건이다.

공통 고지는 `packaging/notices`에 준비한 뒤 CLI·템플릿 팩·preview·각 생성 앱과 최종 산출물의 LICENSES에 포함한다. Rust workspace의 두 지원 target과 빌드 종속성, 웹/패키징 도구 고지를 포함한 확장 목록이므로 실제 Player에 링크된 패키지 목록과 같다고 해석하지 않는다. 목록·원문·출처·보조 고지와 필요한 소스 아카이브를 함께 둔다. 이 묶음은 attribution 자료이며 별도 법률 심사 통과를 뜻하는 필드로 쓰지 않는다.

고지 생성은 릴리스 유지보수 단계다. 제작자는 추가 고지 도구 없이 검증된 팩을 사용한다. 유지보수자는 [cargo-about](https://github.com/EmbarkStudios/cargo-about) 0.9.1을 설치하고 아래 명령으로 재생성한다. 라이선스 파일이 패키지에서 빠진 경우 `packaging/licenses/clarifications.json`의 버전·출처·SHA-256으로 확보한 원문/선언과 원본 소스를 사용하며 제작 데이터에서 권리 정보를 추측하지 않는다.

```sh
cargo install cargo-about --version 0.9.1 --locked --features cli
node scripts/prepare-notices.mjs
node scripts/prepare-notices.mjs --check
```

Builder는 현재 Cargo/웹 잠금 파일·고지 생성 입력과 묶음 파일 hash를 대조하며 불일치 시 NOTICES_STALE/NOTICES_INTEGRITY로 실패한다. CLI 릴리스 패키징과 CI도 같은 검사 단계를 사용한다. 고지·원본 소스 아카이브는 원본 byte hash가 유지되도록 Git 줄바꿈 변환 대상에서 제외한다.
