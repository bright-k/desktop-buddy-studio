# 로컬 Studio 사용과 저장 구조

기준일: 2026-11-07. 현재 구현 계약은 [ADR 0004](decisions/0004-project-studio-and-media.md)와 [리소스 형식](project-format.md)을 따른다.

## 실행

저장소에서 `npm install` 후 `npm run studio`를 실행한다. macOS에서는 루트의 `Studio.command`를 더블클릭해도 된다. Node 22 이상과 고정 Rust 도구체인이 필요하다. 실행 중 CLI 계약과 웹 화면을 빌드하고 검증된 Player 템플릿 팩을 준비한다. 브라우저에 자동으로 로컬 주소를 연다. `BUDDY_STUDIO_OPEN=0`은 자동 열기를 끈다. 기본 포트 4340이 사용 중이면 임의의 빈 포트로 열고 실제 주소를 출력한다. `BUDDY_STUDIO_PORT`, `BUDDY_STUDIO_DATA`로 포트와 데이터 폴더를 지정할 수 있다.

한 데이터 폴더에는 한 Studio만 실행한다. 두 번째 실행은 이미 실행 중인 주소를 안내한다. 실행 주소의 fragment 토큰으로 세션을 만들고 토큰은 주소 표시줄에서 제거한다. 루프백 서비스이며 외부에 공개하지 않는다.

## 제작 흐름

1. 새 프로젝트에서 이름·패키지 ID·템플릿을 선택한다. 패키지 ID가 폴더·URL·앱 identifier가 된다. 내부 projectId는 별도 UUID이며 실행 파일 이름(slug)은 이름에서 제안한다.
2. 리소스 탭에서 PNG/JPEG/GIF/정적 WebP/BMP 이미지와 WAV/MP3 사운드를 가져온다. 파일당 16 MiB, 보관함 256 MiB다. 최종 참조 파일은 전체 64 MiB·디코딩 RGBA 128 MiB 제한을 별도로 적용한다.
3. 애니메이션 탭에서 기본 스프라이트 또는 추가 리소스를 고른다. 스프라이트는 프레임 번호, 사각형 드래그 선택, X/Y/크기 지정, 격자 프레임 생성으로 표시 영역을 정한다. GIF 프레임을 하나씩 확인해 재생 목록에 넣고, 목록의 번호를 편집해 순서와 반복을 정한다. 목록을 비우면 원본 전체를 사용한다. 원본 시간이나 고정 FPS를 선택할 수 있다. **동작 크기와 방향 맞추기**에서 저장된 대기 동작과 나란히 보며 선택 동작의 배율을 50~200%로 조절한다. 100%는 자동 맞춤 크기다. 원본이 보는 방향과 프레임 왼쪽에서 잰 가로 기준점도 동작마다 지정할 수 있다. 기준점 입력을 비워 두면 캐릭터 공통값을 사용한다. 미리보기에서 왼쪽과 오른쪽을 선택해 반전 시 중앙선에 기준점이 맞는지 확인할 수 있다.
4. 대기 상태 탭에서 다른 동작이 재생되지 않을 때의 기본 모습을 고른다. 커서 방향 옵션을 켜면 9구역별로 기본 모습을 상속하거나 반복 동작·이미지·GIF와 좌우 반전을 지정할 수 있다. 리소스를 직접 고르면 전체 파일을 재생하는 반복 동작을 만든다. 기존 대기 방향 이벤트는 이 탭으로 표시되고 저장할 때 새 형식으로 기록된다.
5. 이벤트 탭에서 키 누름/뗌/유지, 입력 속도, 한글·영문 키워드, 마우스 버튼 누름/뗌, 휠, 버디 클릭과 드래그를 연결한다. 버디 위에서 기본 버튼을 누르면 놓을 때 클릭하거나 이동 거리를 넘길 때 드래그한다. 우클릭은 버디 메뉴를 열며 일반 마우스 이벤트를 실행하지 않는다. **키 녹화**를 누른 뒤 Esc나 Ctrl+C처럼 등록할 키를 직접 누른다. **키 추가**로 Cmd+C를 같은 이벤트에 연결할 수 있고, **키워드 추가**로 `춤`과 `dance`를 각 키 배열에 맞춰 같은 이벤트에 연결할 수 있다. 오른쪽 이벤트 테스트에서 각 입력을 선택해 실행한다. 일회성은 loop=false, 속도·키 유지·드래그는 loop=true 동작을 사용한다. 키 유지에는 이동도 선택할 수 있다. 일회성 동작은 우선순위에 따른 전환, 즉시 전환, 완료까지 유지 중에서 고른다. 우선순위는 숫자가 클수록 높다. 같은 우선순위에서는 보조 키 조건을 지정한 이벤트가 먼저고, 조건 지정 여부까지 같으면 목록에서 위에 있는 이벤트가 먼저다. 버디를 드래그하는 동안에는 드래그 동작을 우선 표시한다. 이벤트마다 커서 따라보기와 사운드·볼륨을 설정할 수 있다.
6. 반응 타이밍 탭에서 입력 중단·측정 창·평가 간격·안정 시간을 조절한다. 기본은 250/2000/50/100ms다. 일회성의 표시 시간·키를 떼면 중단 여부·쿨다운은 이벤트에서 정한다.
7. 검증 또는 저장을 누른다. 오른쪽 미리보기는 저장된 버전의 공통 Canvas 렌더러를 사용한다. 이벤트 테스트는 Rust Runtime으로 합성 이벤트를 실행한다. 실제 다른 앱 입력을 읽는 브라우저 기능은 아니다.
8. 앱 빌드를 누른다. 변경이 있으면 먼저 저장한다. 전역 대기열은 한 번에 한 빌드만 실행하며 다른 프로젝트 편집을 계속할 수 있다. 취소, 로그, 완료된 ZIP/설치 파일·보고서 다운로드를 제공한다.
9. 복제는 새 앱 식별자를 만든다. 보관한 프로젝트는 보관 목록에서 복원할 수 있다. 저장 이력에서는 이전 구성·리소스를 함께 복원한다.

현재 macOS 14에서는 개발용 .app·DMG와 편의 ZIP을 생성한다. 정식 macOS 패키지는 macOS 15 이상에서, Windows 설치 파일은 Windows 11 25H2 이상 x64 환경에서 만든다. 생성 앱 실행에는 Node·Rust·Studio가 필요 없다. 빌드 결과를 실행 중인 다른 버전 앱 위에 자동 설치하지 않는다.

## 프로젝트별 보관

```text
.data/
  .studio.lock                  # 실행 중인 서비스 소유권·접속 주소
  .staging/                     # 검증·저장·시뮬레이션 임시 작업
  projects/<package-id>/
    project.json                # 이름·현재 revision·보관 여부
    resources/
      index.json                # 원래 이름·크기·업로드 메타데이터
      <sha256>.<extension>      # 불변 원본 리소스
    revisions/<revision-id>/
      buddy.project.json
      character/manifest.json
      character/sprite.png
      character/resources/...
      character/LICENSE.txt
      assets/app-icon.png
    draft.json                 # 임시 편집 (저장된 구성과 분리)
    drafts/                    # 저장·취소한 임시 편집 보관
    previews/<revision-id>/     # 디코딩된 PNG 프레임 캐시
    jobs/<job-id>.json          # 상태·제한된 로그·결과
    builds/<slug-version-job>/  # 앱·ZIP/설치 파일·보고서·라이선스
    exports/<revision>.tar.gz   # 저장한 정의·원본 리소스의 이동용 묶음
```

저장 중 검증이 실패하면 현재 revision을 교체하지 않는다. 저장 포인터는 atomic rename으로 바꾸며 과거 revision은 유지한다. 앱·프로젝트 ID는 기존 제품에서 수정하지 않고 복제를 사용한다. 리소스 제외는 논리 참조만 제거하며 원본은 보관함에 남는다. 서버 재시작 시 진행 중이던 로컬 빌드는 중단 상태로 표시한다. Node 프로세스 종료 시 활성 로컬 빌드를 취소하고 하위 작업이 종료될 때까지 기다린다. Actions 실행은 재시작 후 다시 연결한다.

엔진 실행 파일과 신뢰하는 템플릿 팩은 `.buddy/studio-engines/<binary-hash>/`에 함께 고정한다. 개발 중 target/debug/buddy가 다시 컴파일돼도 실행 중인 Studio의 빌더와 팩이 섞이지 않는다.

선택한 프로젝트·탭·애니메이션·이벤트는 URL의 project/tab/clip/event query로 유지한다. project는 com.example.typingcat 같은 패키지 ID다. 새로고침은 같은 선택을 열고 임시 편집이 있으면 복원한다. 임시 보관이 완료되지 않은 수정만 브라우저 이탈 안내를 표시한다. 없는 프로젝트·잘못된 query는 유효한 기본 선택으로 복구한다.

## 패키지 ID와 임시 편집

프로젝트 폴더는 `.data/projects/com.example.typingcat/`, 브라우저 주소는 `?project=com.example.typingcat&tab=resources`처럼 같은 키를 쓴다. 생성 화면에서 패키지 ID를 직접 지정하고 이후에는 고정한다. 내부 projectId는 별도 UUID로 유지한다. 기존 UUID 폴더는 시작할 때 패키지 ID로 이전하며 원래 메타데이터와 이전 journal을 보관한다. 충돌 시 덮어쓰지 않고, 리소스·revision·빌드 파일과 앱 식별자는 유지한다. 이전 UUID 주소·다운로드도 연결한다.

편집은 draft.json에 자동 보관한다. 잘못된 값이 있어도 임시 편집을 보존하고 재진입 시 복원한다. 미리보기와 빌드에는 검증하여 저장한 구성만 사용한다. Ctrl/Cmd+S로 저장하며 상단 도구는 스크롤해도 남는다. 브라우저의 뒤로·앞으로 가기와 새로고침은 프로젝트, 탭, 애니메이션과 이벤트 선택을 복원한다. 리소스의 사용 동작과 애니메이션의 연결 이벤트를 클릭해 편집 화면으로 이동할 수 있다. 로컬 서비스 연결이 끊기면 현재 화면을 유지하고 다시 연결 또는 편집 JSON 저장을 제공한다. 이전 임시 편집은 drafts/에 보관한다. [UX 검토 기록](studio-ux-review.md)에 개선과 확인 범위를 정리했다.

## macOS·Windows 빌드

한 프로젝트의 build.targets는 두 플랫폼을 지원한다. 현재 지원 아키텍처는 macOS arm64와 Windows x64이며 Intel Mac·Windows arm64는 포함하지 않는다. 빌드 화면에서 두 플랫폼과 현재 호스트의 빌드 가능 상태를 함께 보여준다. 로컬 빌드 버튼은 현재 OS의 패키지만 만든다. 현재 빌더는 다른 OS로의 교차 컴파일을 허용하지 않는다.

빌드 탭의 ‘이 컴퓨터에서 빌드 · 프로젝트 내보내기’에서 ‘저장한 프로젝트 내보내기’를 누르면 정의·참조 원본·아이콘·라이선스가 포함된 tar.gz를 내려받는다. 내부 앱 ID와 content hash는 유지된다. 저장 전 변경은 먼저 검증·저장하며 임시 편집·이전 빌드·관련 없는 파일은 포함하지 않는다. 이동용 묶음 자체는 실행 앱이 아니다.

내보낸 묶음을 같은 엔진 버전의 소스 저장소 안 `projects/`에 풀고 의존성을 준비한 뒤 다음을 실행한다.

```sh
npm ci --ignore-scripts --no-audit --no-fund
node scripts/build-project.mjs projects/com.example.typingcat --development
```

Mac에서 실행하면 macOS용, Windows x64에서 실행하면 Windows용 결과가 `artifacts/project-builds/<target>/`에 생긴다. 제작 도구체인은 [빌드 문서](build-and-distribution.md)를 따른다. 지원 OS에서 개발 플래그를 빼면 정식 지원 기준을 검사한다. 공개 배포용 인증서·공증 설정은 별도다.

두 결과물을 한 번에 만들려면 내보낸 프로젝트를 저장소에 추가하고 `.github/workflows/build-project.yml`의 **Build project for macOS and Windows**를 수동 실행한다. `project_path`에 `projects/com.example.typingcat`을 지정한다. 같은 커밋의 같은 정의를 macos-15 arm64·windows-2025 x64 runner가 각각 빌드하고, `project-macos-arm64`·`project-windows-x64` 아티팩트를 보관한다. 각 묶음의 source.json·build-report.json에서 identifier·projectHash를 비교할 수 있다.

이 workflow는 개발용 패키지다. Windows hosted runner는 Windows Server이므로 지원 기준인 Windows 11의 대화형 실기 검증을 대신하지 않는다. 수동 workflow 외에 Studio에서 저장소 연결·업로드·실행·취소·결과 가져오기를 수행하는 `.github/workflows/studio-build.yml` 경로도 제공한다. [Actions 연동 안내](studio-actions.md)를 따른다.

## 마우스 따라오기와 설치 화면

- **마우스 따라오기** 탭에서 정지 거리와 거리별 속도·반복 동작을 지정합니다. 거리 0은 캐릭터 중심을 포인터까지 이동시킵니다. 설치된 앱에서는 설정 창이나 버디 우클릭·트레이 메뉴로 기능을 켜고 끌 수 있습니다. 기본 정지 거리는 48 DIP이며 버디가 있는 화면 안에서 따라옵니다. 따라오기 우선순위를 지정하면 같은 값이나 더 높은 값의 이벤트가 애니메이션을 바꿀 수 있습니다.
- macOS 빌드는 기본 배경과 PDF를 포함합니다. DMG를 열면 왼쪽부터 가이드, 앱, Applications가 한 줄에 표시됩니다. 빌드 탭의 **macOS 설치 화면**에서 배경, 창 크기와 아이콘 위치를 조정합니다. **설치 가이드**에서 가이드를 교체하거나 기본 PDF로 되돌릴 수 있습니다. 배경은 PNG·JPEG, 가이드는 PDF·PNG·JPEG 또는 이미지가 포함된 단일 UTF-8 HTML을 지원합니다. 기본 파일과 배치 값은 [packaging/macos/defaults](../packaging/macos/defaults)에 있습니다.
- Windows 빌드에서 **Windows 설치 패키지 (ZIP)**를 받으면 설치 파일, [Windows 기본 PDF](../packaging/windows/defaults/installation-guide.pdf), 라이선스를 함께 받습니다. 직접 올린 가이드는 두 플랫폼에서 사용합니다. macOS 가이드를 제외하면 Windows는 기본 PDF를 사용합니다.
- 설치한 버디의 **설정 · 입력 권한 → 자동 실행**에서 **로그인할 때 버디 실행**을 켜거나 끌 수 있습니다. 변경은 바로 적용되며, 켜면 컴퓨터를 켜거나 재시작한 뒤 현재 사용자 계정에 로그인할 때 실행됩니다. macOS가 승인을 요청하면 시스템 설정의 로그인 항목에서 허용하세요.
- 전체 설정 초기화는 버디 설정에서 확인 후 실행합니다. 위치만 초기화하면 편집 중인 다른 설정은 유지됩니다.
- 프로젝트 탭의 **기본 크기 기준**에서 배율 또는 투명 여백을 뺀 대기 애니메이션의 표시 높이(DIP)를 선택합니다. 버디를 옮길 때 화면 가장자리에 모든 동작이 들어갈 공간과 8 DIP를 확보합니다.
- 빌드 탭에서 완료된 빌드의 **기록 삭제**를 누른 뒤 기록만 지우거나 Studio가 보관한 해당 빌드 파일까지 지울 수 있습니다. 이미 내려받은 복사본과 GitHub Actions는 유지됩니다.
