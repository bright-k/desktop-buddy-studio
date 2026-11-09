# Studio의 GitHub Actions 연동

## 사용 흐름

1. 빌드 탭에서 `owner/repository`를 연결한다. 직접 입력 토큰이 우선이며, 비워두면 `GH_TOKEN`·`GITHUB_TOKEN`·Git credential helper 순으로 인증을 찾는다.
2. ‘워크플로 준비’를 누르면 기본 브랜치에 `.github/workflows/studio-build.yml` 한 파일을 추가/업데이트한다. 기존 파일의 blob SHA로 충돌을 검사한다. 브랜치 보호·조직 정책을 우회하지 않는다.
3. 저장소 공개 여부와 전송 범위를 확인하고 **업로드하고 두 플랫폼 빌드**를 누른다. 변경은 먼저 검증·저장한다. 저장된 정의·참조 원본·아이콘·라이선스·고정 엔진을 별도 브랜치에 보낸다. 임시 편집·다른 프로젝트·이전 빌드·자격 증명·입력 이력은 포함하지 않는다.
4. macOS arm64·Windows x64 작업을 함께 실행한다. Studio에서 플랫폼별 단계·성공·실패·취소를 확인하고 상세 로그는 Actions 링크로 연다. 편집을 계속해도 실행 중인 빌드 구성은 고정된다.
5. 성공한 플랫폼의 결과를 검증한 뒤 `builds/actions-<jobId>/`에 보관한다. Windows 설치 ZIP, macOS DMG, 전체 tar.gz와 영수증·보고서를 내려받는다. 한 플랫폼만 성공해도 보존한다.

공개 저장소는 원본 리소스도 공개된다. 비공개 리소스는 접근을 제한한 저장소를 선택한다. 비공개 저장소의 실행 시간·보관은 GitHub 요금제 한도를 따른다. Studio는 저장소 공개 범위·과금·조직 정책을 변경하지 않는다.

## 인증

선택한 저장소의 Contents·Actions·Workflows 읽기/쓰기 권한이 필요하다. fine-grained 토큰은 해당 저장소만 선택할 수 있다. classic 토큰은 repo/workflow scope가 필요하다. 실제 허용 여부는 API 응답으로 검사한다.

직접 입력 토큰은 Node 서비스 메모리에만 보관하고 요청 후 브라우저 입력을 비운다. `.data/github.json`에는 저장소·인증 방식만 쓴다. URL·localStorage·프로젝트·로그·보고서에 자격 증명을 넣지 않는다. 기존 Git/환경 변수 인증은 재시작 때 재사용할 수 있고 직접 입력 토큰은 다시 연결해야 한다. 연결 해제는 원격 실행 자체를 취소하지 않는다.

API origin은 `https://api.github.com`으로 고정한다. artifact 리디렉션은 GitHub/Actions의 HTTPS 저장소만 허용하고 Authorization을 전달하지 않는다. 연결 API에도 기존 Studio 세션·Host·Origin 검사를 적용한다.

## 고정 버전과 무결성

기본 브랜치에는 workflow만 준비한다. `studio-engine/<manifest-sha256>`에는 내장 manifest로 검증한 소스·잠금 파일·고지를, `studio-build/<job-uuid>`에는 같은 엔진과 `.studio/project`·request.json을 커밋한다. Git tree API로 허용 파일만 전송한다. 로컬 Git index·작업 파일은 바꾸지 않는다. 원격 브랜치·커밋은 재현성을 위해 자동 삭제하지 않는다.

기존 엔진 branch는 전체 파일 수·모드·blob hash가 일치해야 재사용한다. 동일 요청 branch의 다른 tree는 덮어쓰지 않는다. `.gitattributes`로 원격 체크아웃의 줄바꿈 변환을 막는다. workflow는 엔진 manifest SHA-256·개별 파일 hash·요청 정보를 먼저 검증하고, CLI는 project hash가 다르면 중단한다. 프로젝트가 제공한 스크립트는 실행하지 않는다. checkout 자격 증명은 남기지 않고 workflow 기본 권한은 contents:read다.

가져오기는 artifact digest → ZIP CRC·크기·허용 파일명 → job·플랫폼·project/engine hash 영수증 → tar 내부 source.json/build-report.json → 설치 파일·Windows 설치 ZIP·가이드의 보고서 SHA-256 순으로 검사한다. ZIP/tar 경로를 그대로 파일시스템에 풀지 않는다. 응답·디코딩 상한은 각각 256 MiB다. `.app` 권한은 tar.gz 안에 보존한다.

Windows Builder는 Node 도구에 일반 드라이브/UNC 경로를 전달한다. MSVC의 긴 출력 경로 제한을 피하려고 중간 Cargo 캐시만 짧은 임시 폴더 아래에 두며 엔진·target·출력 폴더 hash로 격리한다. `BUDDY_BUILD_CACHE`로 짧은 캐시 루트를 지정할 수 있다. 최종 앱·보고서는 프로젝트 출력 폴더에 유지한다.

## 중복·재시작·오류

같은 저장소·프로젝트·revision·엔진의 진행 요청은 재사용한다. dispatch 전에 시도를 기록하고 `return_run_details`로 실행 ID를 받는다. 응답이 끊기면 고유 branch·commit으로 기존 실행을 찾고 자동 dispatch를 반복하지 않는다. 확정 거부는 실패로 표시하고 새 요청을 허용한다.

Studio 종료는 원격 실행을 취소하지 않는다. 재시작·재연결 후 같은 저장소의 진행 요청을 다시 확인한다. 인증 만료·저장소 변경은 연결 대기로 남긴다. 실행 ID·branch·commit이 다른 결과는 가져오지 않는다. GitHub에서 직접 재실행해 attempt가 달라지면 새 Studio 빌드를 안내한다.

‘빌드 취소’는 해당 요청만 취소한다. 업로드 중에는 취소를 먼저 기록하고 진행 중인 요청이 돌아오면 다음 업로드·브랜치 생성·실행 요청을 건너뛴다. 이미 실행을 요청했으면 GitHub에 취소를 보내며, ID가 불확실하면 기존 실행을 찾아 취소한다. 이미 업로드된 객체는 삭제하지 않는다. ‘결과 다시 가져오기’는 기존 실행을 재조회하며 새 runner를 시작하지 않는다. 부분 성공·다운로드 실패를 구분하고 성공한 결과를 유지한다. 빌드 이력에서 OS별 설치 파일을 바로 내려받고, 압축 패키지·영수증·보고서 및 로그는 필요한 경우 펼친다.

## 지원 범위

대상은 macOS arm64·Windows x64다. Intel Mac은 포함하지 않는다. hosted Windows Server는 Windows 11 대화형 입력·설치 검증을 대신하지 않으므로 결과는 모두 개발용이다. 공개 배포 서명·공증·사용자 입력 권한은 별도다. 실제 실행은 [검증 기록](validation/2026-11-07-actions.md)에 둔다.

근거: [workflow dispatch](https://docs.github.com/en/rest/actions/workflows#create-a-workflow-dispatch-event), [Git trees](https://docs.github.com/en/rest/git/trees), [artifact API](https://docs.github.com/en/rest/actions/artifacts).
