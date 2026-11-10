# 프로젝트와 캐릭터 리소스 형식

상태: v1 계약·Rust 타입·검증기 구현, OS 실증 진행 중 / 기준일: 2026-11-07

이 문서의 JSON은 v1 계약 예시다. [샘플 프로젝트](../examples/typing-cat/buddy.project.json), [캐릭터 manifest](../examples/typing-cat/character/manifest.json), [생성 스키마](../packages/contracts/generated/project.schema.json)와 Rust CLI가 존재한다. 문서 예시에서 샘플을 생성하는 scripts/generate-examples.py와 Rust 타입에서 계약을 생성하는 buddy export-contracts를 사용한다. 실제 검증 범위는 [개발 검증 기록](validation/2026-11-07-development.md)에 둔다.

## 1. 프로젝트 단위

프로젝트 하나가 독립 앱 하나를 정의한다. 여러 프로젝트를 같은 소스 트리에 두어도 된다. 앱별 사용자 설정은 프로젝트 원본이 아니라 설치된 앱의 데이터 디렉토리에 저장한다.

CLI에 디렉토리를 주면 buddy.project.json을 진입 파일로 사용한다. 파일 경로를 명시하면 다른 이름의 JSON도 사용할 수 있으며 상대 자산 경로는 그 파일이 있는 폴더를 기준으로 해석한다. 검증 오류는 실제 진입 파일명을 가리킨다. snapshot·미리보기·생성 앱에서도 그 파일명을 보존하며, 파일명 변경은 content hash에 반영된다. 제작 프로젝트에 Player의 project-entry.json descriptor를 직접 작성할 필요는 없다.

```text
typing-cat/
  buddy.project.json
  assets/
    app-icon.png
  character/
    manifest.json
    sprite.png
    LICENSE.txt
```

## 2. 프로젝트 정의 예시

```json
{
  "schemaVersion": 1,
  "projectId": "38142fc3-7b15-4fdb-8cde-51b7e5e3373b",
  "product": {
    "slug": "typing-cat",
    "name": "Typing Cat",
    "version": "0.1.0",
    "identifier": "com.example.buddy.typingcat",
    "icon": "assets/app-icon.png"
  },
  "character": "character/manifest.json",
  "defaults": {
    "scale": 1.0,
    "alwaysOnTop": true,
    "position": { "monitor": "primary", "anchor": "bottomRight", "marginDip": 24 },
    "input": { "keyboard": true, "mouse": true },
    "movementEnabled": false
  },
  "bindings": [
    {
      "id": "typing-slow",
      "trigger": { "type": "typingRate", "minKps": 0.5, "maxKps": 3.0 },
      "effect": { "type": "sustainAnimation", "animation": "typing" }
    },
    {
      "id": "typing-fast",
      "trigger": { "type": "typingRate", "minKps": 3.0 },
      "effect": { "type": "sustainAnimation", "animation": "typingFast" }
    },
    {
      "id": "jump",
      "trigger": { "type": "keyDown", "code": "Space", "repeat": false },
      "effect": { "type": "playAnimation", "animation": "jump", "priority": 80, "retrigger": "restart", "cooldownMs": 120 }
    },
    {
      "id": "click",
      "trigger": { "type": "mouseDown", "button": "primary" },
      "effect": { "type": "playAnimation", "animation": "click", "priority": 60, "retrigger": "restart", "cooldownMs": 100 }
    },
    {
      "id": "secret-cat",
      "trigger": { "type": "keySequence", "keyword": "고양이", "layout": "ko-KR-2set", "timeoutMs": 5000 },
      "effect": { "type": "playAnimation", "animation": "celebrate", "priority": 95, "retrigger": "restart", "cooldownMs": 2000 }
    },
    {
      "id": "right",
      "trigger": { "type": "whileKeyDown", "code": "ArrowRight" },
      "effect": { "type": "move", "x": 1, "y": 0, "speedDipPerSecond": 120, "animation": "walk" }
    }
  ],
  "build": {
    "targets": ["windows-x64", "macos-arm64"],
    "signing": { "windows": "none", "macos": "ad-hoc" }
  }
}
```

키 입력 속도가 0.5 이상 3 KPS 미만이면 typing, 3 KPS 이상이면 typingFast를 기본 애니메이션으로 선택한다. Space의 jump와 `고양이`의 두벌식 물리 키 순서에 맞춘 celebrate는 일회성 반응이다. 세 기능 모두 캐릭터 앱 포커스 없이 다른 앱에서 작업하는 동안 동작한다. 현재 IME의 실제 완성 문자열은 읽지 않는다. 구체적인 속도·한글 변환·키 순서 계약은 [전역 입력 속도와 키워드](input-and-keywords.md)에 둔다.

`movementEnabled`가 false이면 이동과 연결된 walk를 모두 실행하지 않는다. 사용자가 설정에서 켜고 오른쪽 키를 새로 누르면 이동하며, 속도 애니메이션이 활성화되지 않은 경우 walk가 반복된다. 이동 중 Space를 누르면 이동은 계속되고 jump가 화면에 우선 표시된다. jump가 끝나면 그 시점의 속도 애니메이션·walk·idle 순서로 기본 상태를 처음부터 재생한다. 감지한 입력은 원래 앱에도 전달된다.

### 필드 계약

| 필드 | v1 규칙 |
| --- | --- |
| `schemaVersion` | 정수 1. 미래 버전은 명시적인 마이그레이션 없이는 빌드하지 않는다. |
| `projectId` | UUID. 편집기에서 프로젝트를 식별한다. 새 앱으로 복제하면 새 값이다. |
| `product.slug` | 소문자 영문·숫자·하이픈, 2~64자. 양 끝은 영문·숫자. 실행 파일·산출물 이름이며 같은 제품 업데이트에서 유지한다. |
| `product.name` | 표시 이름, 1~80자·UTF-8 최대 200 bytes. 앱/설치 파일에도 쓰므로 경로 구분자·제어 문자·Windows 예약 문자/이름·끝 공백/점을 금지한다. 한글·일반 공백·작은따옴표는 허용한다. |
| `product.version` | `major.minor.patch`, v1 제품 계약은 각 성분 정수 0~99, 0 외의 선행 0·접미사 금지. M0에서 경계값의 플랫폼 패키지 변환을 검증한다. |
| `product.identifier` | 소문자 영문으로 시작하는 영문·숫자 segment를 점으로 구분, 3개 이상 segment, 전체 3~128자. 새 프로젝트 기본값은 `io.desktopbuddy.app.p` + projectId의 하이픈 없는 소문자 hex다. 제품마다 유지한다. |
| `product.icon` | 프로젝트 루트 기준 PNG 경로. 빌더가 OS별 아이콘 파일을 생성한다. |
| `character` | 프로젝트 루트 기준 캐릭터 manifest 경로. |
| `build.targets` | 허용 목록 값만 받는다. 대상 지원 여부는 빌드 호스트 검사와 별개다. |
| `build.signing` | Windows `none` 또는 `self-signed`, macOS `ad-hoc` 또는 `self-signed`. 키 정보는 넣지 않는다. |

`product.identifier`는 같은 앱 업데이트에서 유지한다. 새 이름으로 복제한 앱에는 새 식별자를 만든다. 같은 식별자와 다른 프로젝트를 함께 빌드하면 충돌을 경고하고 독립 제품으로 배포하지 못하게 한다. Builder는 같은 출력 디렉토리의 이전 성공 보고서를 검사해 다른 projectId의 identifier 재사용과 동일 identifier의 slug 변경을 거부한다. 다른 출력 폴더·컴퓨터 전체의 식별자 유일성을 보장하지는 않는다.

`product.name`은 업데이트에서 바꿀 수 있다. identifier·slug 변경은 새 제품으로 취급하며 자동 데이터 이전을 하지 않는다. icon은 정사각형 PNG, 한 변 512~1024 px로 제한하고 일반 이미지 한도도 적용한다.

필수 필드는 `schemaVersion`, `projectId`, `product`, `character`다. 나머지 생략값은 이 문서의 defaults와 동일한 기본값, bindings 빈 배열, 현재 호스트의 지원 대상, Windows none/macOS ad-hoc로 정규화한다. 명시한 필드에 잘못된 값이 있으면 기본값으로 조용히 대체하지 않는다.

defaults의 부분 객체도 필드별로 기본값을 채운다. scale은 0.25~3.0이다. 선택 항목 `displayHeightDip`(16~1024)가 있으면 배율 대신 기본 대기 모션의 투명 여백을 뺀 높이를 DIP로 맞춘다. 설치 앱의 사용자 크기 100%는 이 높이를 기준으로 한다. alwaysOnTop·입력 토글·movementEnabled는 boolean이다. position.monitor는 v1에서 `primary`, anchor는 `topLeft`/`topRight`/`bottomLeft`/`bottomRight`, marginDip은 0~256(기본 10)이다. 처음 배치할 때는 캐릭터가 실제로 보이는 부분을 기준으로 하고, 화면 가장자리에는 최소 6 DIP를 둔다. 실제 모니터 선택과 로컬 DIP 좌표는 사용자 preferences에 저장한다. 화면보다 큰 경우 Host의 작업 영역 제한이 우선한다. binding.id는 프로젝트 안에서 고유한 1~64자 영문·숫자·하이픈·밑줄이다. 숫자는 유한값만 허용한다.

## 3. 캐릭터 manifest 예시

```json
{
  "schemaVersion": 1,
  "id": "sample-cat",
  "name": "Sample Cat",
  "license": "LICENSE.txt",
  "image": "sprite.png",
  "sheet": { "width": 1024, "height": 1024, "frameWidth": 256, "frameHeight": 256, "frameCount": 16 },
  "anchor": { "x": 0.5, "y": 1.0 },
  "defaultAnimation": "idle",
  "animations": {
    "idle": { "frames": [0, 1], "fps": 2, "loop": true },
    "typing": { "frames": [2, 3], "fps": 12, "loop": true },
    "typingFast": { "frames": [14, 15], "fps": 24, "loop": true },
    "click": { "frames": [4, 5], "fps": 12, "loop": false },
    "jump": { "frames": [6, 7, 8, 9], "fps": 12, "loop": false },
    "walk": { "frames": [10, 11, 12, 13], "fps": 8, "loop": true },
    "celebrate": { "frames": [6, 7, 8, 9], "fps": 12, "loop": false }
  }
}
```

프레임은 왼쪽 위부터 행 우선, 0부터 시작한다. 각 프레임 크기와 anchor를 동일하게 유지해 동작 전환 시 캐릭터가 튀지 않도록 한다. `defaultAnimation`은 대기 상태에 사용할 기존 반복 애니메이션의 이름이다. FPS는 정수 1~30, anchor는 각 축 0~1이다. 애니메이션 이름은 1~64자 영문·숫자·하이픈·밑줄, 애니메이션은 최대 128개, frames는 비어 있지 않은 최대 256개 정수 배열이다. 프레임 반복 참조는 허용한다.

manifest의 이미지·라이선스 경로는 manifest가 있는 디렉토리 기준이다. 프로젝트의 다른 자산 경로 기준과 혼동하지 않는다. 이름만 적은 라이선스를 권리 증명으로 해석하지 않으며 제작자가 사용 권한을 확보한 리소스를 포함한다.

모든 동작이 `resource`로 이미지나 GIF를 참조하면 manifest의 `image`를 생략할 수 있다. 이 경우 `sheet.frameWidth`와 `sheet.frameHeight`를 표시 기준으로 사용한다. Studio는 첫 스프라이트를 기다리는 빈 프로젝트를 저장할 수 있다. 미리보기와 빌드에는 반복 재생되는 `defaultAnimation`이 필요하다.

## 4. 이벤트·행동 계약

| Trigger type | 허용 effect | 의미 |
| --- | --- | --- |
| `keyDown`, `keyUp` | `playAnimation` | 물리 키와 선택적 보조 키 조합. `*`는 지원 목록 중 보조 키를 제외한 일반 키 전체 |
| `whileKeyDown` | `move`, `sustainAnimation` | 물리 키와 지정한 조합을 유지하는 동안 이동 또는 반복 동작. `*` 금지 |
| `mouseDown`, `mouseUp` | `playAnimation` | 전역 `primary`, `secondary`, `middle` 버튼. 버디 위에서 시작한 기본 버튼 입력은 버디가 처리한다. |
| `buddyClick` | `playAnimation` | 버디 위에서 기본 버튼을 눌렀다가 드래그 없이 놓았을 때 한 번 실행. |
| `wheel` | `playAnimation` | `up`, `down`, `left`, `right` 방향. OS 델타는 정규화한다. |
| `mouseInput` | `playAnimation` | `inputs`의 버튼 누름·뗌, 스크롤 방향, 버디 클릭 중 하나가 들어오면 실행. |
| `typingRate` | `sustainAnimation` | 설정한 측정 창(기본 2초)의 실제 키 입력 속도 구간에 머무르는 동안 반복 애니메이션 |
| `keySequence` | `playAnimation` | keyword+layout으로 변환한 물리 키 순서 또는 명시적 steps의 매칭 |
| `drag` | `sustainAnimation` | 버디를 직접 드래그하는 동안 반복 동작. 놓으면 즉시 해제 |
| `dragDropped` | `playAnimation` | 직접 드래그한 위치를 성공적으로 저장했을 때 한 번 실행 |
| `firstShown`, `shownAgain` | `playAnimation` | 첫 표시 또는 숨겼다가 다시 표시한 순간에 한 번 실행 |
| `idleFor` | `playAnimation` | 버디의 대기 상태가 `seconds`(1~3600)만큼 이어졌을 때 실행 |
| `typingStopped` | `playAnimation` | 일정 시간 타이핑한 뒤 마지막 유효 키 입력에서 멈춤 판정 시간이 지나면 실행 |

typingRate.minKps는 0.5~1000, maxKps는 minKps보다 큰 1000 이하 값이며 생략하면 상한 없음이다. 구간은 `[minKps, maxKps)`이고 프로젝트당 최대 8개, 서로 겹치면 검증 오류다. sustainAnimation은 loop=true인 animation을 참조한다. 키 유지·속도 동작과 이동은 priority를 정수 0~100으로 지정할 수 있다. 생략하면 일회성 → 키 유지 → 속도 → 이동 순서가 적용된다. 드래그 중에는 편집용 동작을 표시한다.

`mouseInput.inputs`에는 서로 다른 `mouseDown`, `mouseUp`, `wheel`, `buddyClick` 조건을 1~11개 지정할 수 있다. 조건은 이벤트의 재반응 대기 시간, 재실행 정책과 우선순위를 공유한다. 모든 조건이 `mouseDown`이면 반응을 시작한 버튼을 뗄 때 재생을 중단하도록 설정할 수 있다. 기존 단일 조건 형식도 읽는다.

키 이벤트의 `modifiers`에는 `control`, `alt`, `shift`, `meta`를 중복 없이 지정할 수 있다. Ctrl+C는 `code: "KeyC", modifiers: ["control"]`, macOS의 Cmd+V는 `code: "KeyV", modifiers: ["meta"]`로 설정한다. 좌우 보조 키는 같은 조건으로 처리한다. 배열을 지정하면 누른 보조 키가 배열과 정확히 일치해야 하며, 빈 배열은 보조 키가 없는 입력에 해당한다. 생략하면 기존처럼 보조 키 상태와 상관없이 매칭한다. 키 뗌은 주 키를 뗄 때, 키 유지는 유지하는 동안 조건을 검사한다. 같은 우선순위에서는 보조 키 조건을 지정한 이벤트가 먼저다.

같은 키 이벤트에 `alternatives` 배열로 `{ code, modifiers }`를 더할 수 있다. 키워드 이벤트의 `alternatives`에는 `{ keyword, layout }`를 넣어 입력마다 키 배열을 선택한다. 기본 입력과 추가 입력 중 하나만 일치해도 같은 이벤트가 실행된다. 재반응 대기 시간, 반복 입력 옵션, 키워드 입력 제한 시간은 이벤트 단위로 적용한다. 한 키 이벤트에는 최대 32개 입력을 둘 수 있고, 키워드는 추가 입력을 포함해 프로젝트 전체에서 최대 32개를 쓸 수 있다.

keySequence는 keyword+layout 또는 steps 중 하나만 가진다. keyword는 NFC 후 1~32 Unicode scalar, layout은 ko-KR-2set/en-US-qwerty다. steps는 1~128개의 `{ code, shift }`이며 shift는 필수 boolean이다. timeoutMs는 정수 500~10000, 기본 5000이다. sequence는 최대 32개, 변환 후 동일 패턴 중복은 오류다. 지원 문자·두벌식 변환표·중단 키·겹침 규칙은 [입력 계약](input-and-keywords.md)이 기준이다. 한 이벤트에서 키·wildcard·완성 sequence 후보를 함께 비교한다.

### 일회성 반응

- `playAnimation`이 참조하는 animation.loop는 false여야 한다. priority는 정수 0~100(기본 50), cooldownMs는 정수 0~10000(기본 0), retrigger는 restart/ignore(기본 restart)다. animation 참조는 필수다.
- 선택 필드 `variation`의 `mode`는 `cycle`, `shuffle`, `weighted` 중 하나다. `alternatives`에는 1~7개의 애니메이션 ID를 지정한다. 첫 후보는 `animation`이다. 전체 후보는 서로 다른 일회성 애니메이션이어야 한다. `cycle`은 목록 순서대로 반복하고, `shuffle`은 한 라운드에서 각 후보를 한 번씩 사용하고 라운드 경계에서도 같은 후보가 연속되지 않도록 섞는다.
- `weighted`는 `weights`에 첫 후보부터 순서대로 후보당 하나의 정수(0~10000)를 요구한다. 최소 한 값은 양수여야 한다. 매번 후보의 비중을 전체 합계로 나눈 확률로 독립 추첨한다. 비중 0은 선택에서 제외된다. `cycle`과 `shuffle`에 보관된 비중은 재생에 사용하지 않는다.
- 선택 순서는 실행 중 이벤트별로 관리하고, 실제로 반응을 시작했을 때만 전진한다. 숨김·다시 표시 뒤에도 순서는 이어진다. 후보는 이벤트에 설정한 사운드·우선순위·쿨다운을 공유한다. `durationMs`를 생략하면 선택된 애니메이션의 길이를 사용한다.
- 한 이벤트의 적격 후보는 priority 내림차순, bindings 배열 순서 오름차순으로 하나를 선택한다. 이벤트 간에는 정규화 seq 순서를 따른다. 적격 후보가 없으면 아무것도 시작하지 않는다.
- 현재 재생 중인 동작의 interruption이 byPriority(기본)이면 더 높은 우선순위 또는 같은 우선순위에서 앞선 정의가 교체한다. replace이면 새로 시작한 적격 이벤트가 즉시 교체한다. finish이면 원래 종료 시각까지 새 이벤트와 반복 입력을 무시한다. 그 외의 동일 이벤트 반복은 retrigger를 따른다.
- 쿨다운은 binding별 마지막 실제 시작 시각부터 잰다. 쿨다운·우선순위·ignore로 버린 입력은 시각을 갱신하지 않는다. 중단된 반응은 재개·대기열 저장하지 않는다.
- Rust가 정규화된 프레임 시간 합계 또는 지정한 durationMs 후 종료한다. finish 이외의 동작은 stopOnRelease가 켜져 있으면 해당 키나 버튼을 떼는 시점에도 중단한다. 렌더러 ACK는 완료 조건이 아니다. 종료 시 현재 활성화된 유지 조건의 우선순위를 다시 적용한다. 드래그 중에는 drag 동작이 우선한다.

첫 표시·다시 표시·대기 상태 지속은 입력 반응과 실제 이동에 양보하는 자동 동작이다. 시작할 때 다른 동작이 활성 상태면 해당 차례를 건너뛴다. 숨김·일시정지·편집·설정 중에는 실행하지 않는다. 시선·반전·방향별 대기 애니메이션의 변화는 대기 시간에 포함한다. 실제 이동과 입력 반응이 끝나거나 버디를 다시 활성화한 뒤에는 대기 시간을 새로 센다. 대기 이벤트가 재생되면 같은 이벤트의 다음 간격은 동작이 끝나는 시각부터 센다. 여러 대기 이벤트의 시간이 동시에 되면 기존 우선순위와 목록 순서로 하나를 고른다.

`typingStopped`는 프로젝트당 하나다. Studio는 `minTypingSeconds` 8(2~60), `minKeys` 12(2~100), `pauseMs` 1500(500~5000), `minIntervalSeconds` 45(10~600)를 기본값으로 채운다. 첫 키와 마지막 키 사이의 시간과 키 수가 모두 기준을 채워야 한다. 글자·숫자·공백·문장부호 물리 키를 세고, 자동 반복·Ctrl/Alt/Meta 조합·탐색 키는 건너뛴다. 같은 입력 배치에서 판정 시각에 유효 키가 들어오면 구간을 이어 센다. 입력 감지가 꺼지거나 재시작되거나 버디를 숨기면 진행 중인 구간을 폐기한다.

타이핑 종료는 속도 애니메이션의 중단 시간과 별도로 평가한다. 입력 반응·드래그·실제 이동이 우선하며, 대기 상태 지속과 동시에 실행할 수 있으면 타이핑 종료를 먼저 선택한다. 최소 재반응 간격은 실제 재생을 시작한 시각부터 센다. 실행할 수 없었던 회차와 처리 시점이 판정보다 500ms 넘게 늦어진 회차는 폐기한다.

### 유지 이동과 방향

move의 x/y는 -1~1, 0 벡터는 금지하고 speedDipPerSecond는 1~1000이다. animation은 생략 가능하며 지정했다면 loop=true인 애니메이션만 참조한다. idle은 이동 애니메이션으로 지정하지 않는다. 같은 키로 여러 binding을 만들 수 있으며 모든 활성 move의 축을 합산해 각 축을 -1~1로 제한한다. 벡터 길이가 1을 넘으면 정규화하고, 활성 move 중 가장 큰 speed를 곱한다. 반대 방향이 상쇄되거나 작업 영역 경계에서 실제 이동이 0이면 이동 애니메이션을 표시하지 않는다.

실제 이동이 있으면 활성 move 중 우선순위가 높은 동작을 후보로 삼는다. 기본 우선순위가 같은 이동은 애니메이션이 지정된 정의가 우선하고, 선택한 이동에 애니메이션이 없으면 기본 대기 모습을 사용한다. 일회성 반응 중에도 이동 계산은 계속한다. 편집·일시 정지에서는 모든 입력 동작을 중지하고, movementEnabled=false는 이동과 walk만 중지한다. 범용 effect 배열과 유지 조건의 독립 playAnimation은 v1에 넣지 않는다.

프로젝트의 선택 필드 `idle`에서 `cursorEnabled: true`이면, 대기 중 캐릭터 영역을 3×3으로 나눠 최신 커서가 있는 구역을 선택한다. 밖에 있는 커서는 가장 가까운 행과 열에 대응한다. `idle.directions`의 키는 upLeft, up, upRight, left, center, right, downLeft, down, downRight다. 각 키에 반복 `animation`과 `mirror`를 따로 지정할 수 있다. 비어 있으면 기본 대기 동작을 쓴다. 자동 방향은 왼쪽 열을 왼쪽 보기로, 나머지 칸을 오른쪽 보기로 처리하며 동작의 원본 방향에 맞춰 반전한다. `mirror`를 지정하면 원본 또는 반전을 직접 선택한다. 다른 이벤트는 binding의 `followCursor`를 켠 경우에만 좌우로 커서를 따라본다. 이전 프로젝트의 `cursorMove`와 `faceCursor` 조합은 `idle`이 생략된 경우에 대기 방향 설정으로 적용된다.

`cursorFollow.priority`는 생략하거나 0~100으로 지정한다. 따라오는 중에는 이 값 이상인 이벤트가 애니메이션을 바꿀 수 있고 이동은 계속한다. 이미 재생 중인 일회성 이벤트는 자신의 전환 규칙을 따른다. 생략하면 일회성 이벤트와 따라오기, 지속 이벤트 순으로 표시한다. 키로 직접 이동하거나 드래그할 때는 마우스 따라오기를 일시 중지한다.

### 이벤트 사운드

캐릭터의 sounds에 사운드 ID와 manifest 기준 상대 경로 audio를 저장한다. 최대 32개의 WAV 또는 MP3를 추가할 수 있다. binding의 선택 필드 sound는 resource에 사운드 ID를, volume에 0~1 사이의 값을 가진다. 화면에 표시되는 동작이 시작될 때 한 번 재생하고 다른 동작으로 바뀌거나 숨기면 멈춘다. 우선순위에 밀려 시작하지 않은 이벤트에는 소리가 나지 않는다.

### 정규 입력 목록

| code | Windows x64 | macOS arm64 |
| --- | --- | --- |
| `KeyA`~`KeyZ`, `Digit0`~`Digit9`, `F1`~`F12` | 지원 | 지원 |
| `Escape`, `Tab`, `Space`, `Enter`, `Backspace`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `ArrowLeft/Right/Up/Down` | 지원 | 지원, 해당 물리 키가 있는 장치 기준 |
| `ShiftLeft/Right`, `ControlLeft/Right`, `AltLeft/Right`, `MetaLeft/Right` | 지원 | 지원, Meta는 Command·Alt는 Option |
| `Backquote`, `Minus`, `Equal`, `BracketLeft/Right`, `Backslash`, `Semicolon`, `Quote`, `Comma`, `Period`, `Slash` | 지원 | 지원 |

위 표에서 `/`는 각각의 code를 축약한 표기다. 예: ArrowLeft, ArrowRight, ArrowUp, ArrowDown. 물리 키를 기준으로 하며 자판에 인쇄된 문자·IME 결과와 다를 수 있다. `*`는 modifier 8개를 제외한 위 목록만 매칭한다. CapsLock·국제 전용 키·Insert·숫자패드·F13 이상·멀티미디어 키는 v1 매핑 목록에서 제외하고 이벤트는 Unknown으로 무시한다. 문자 추정·임의 코드 등록은 금지한다. 앱별 입력 제외는 P1 이후다.

repeat는 keyDown에서만 허용하며 기본 false다. true이면 최초 down과 OS 반복 down을 모두 후보로 삼는다. repeat는 held 집합에 중복 추가하지 않으며 reset 이후 새 held 상태를 만들지 않는다. keyUp에는 repeat 필드를 받지 않는다. 마우스 primary/secondary는 OS 기본/보조 버튼 설정에 맞추고 middle을 별도로 처리한다. 버디에서 시작한 우클릭은 버디 메뉴에만 전달한다. wheel은 OS 논리 스크롤을 X 양수=오른쪽, Y 양수=아래로 정규화하고, 각 0이 아닌 축을 X→Y 순서로 평가한다. 한 스크롤 입력의 두 축이 같은 이벤트에 해당해도 그 이벤트는 한 번만 시작한다. 어댑터가 노치·연속 델타를 구분하며 동일 물리 휠 거리나 이벤트 횟수를 OS 간 동일하다고 보장하지 않는다.

## 5. 검증과 자산 한도

아래는 v1 검증 한도다. 실측으로 변경이 필요하면 ADR·스키마·fixture를 함께 갱신한다.

- JSON은 UTF-8, 각 JSON 최대 1 MiB, 중복 JSON 필드 금지, 총 매핑 최대 256개다. license 텍스트도 UTF-8, 파일당 1 MiB 이하로 제한한다.
- 기본 스프라이트와 앱 아이콘은 PNG다. 추가 resources는 PNG/JPEG/GIF/정적 WebP/BMP를 지원한다. 파일 1개 16 MiB, 한 변 4096 px, 프로젝트 전체 디코딩 RGBA 128 MiB, 참조 자산 합계 64 MiB 이하로 제한한다. GIF는 최대 256프레임이며 애니메이션 WebP/APNG·SVG는 거부한다.
- 프레임 한 변 16~512 px, frameCount 1~256, 시트의 실제 이미지 크기와 선언값이 일치해야 한다.
- 시트 크기는 프레임 크기의 정수 배이며 `1 <= frameCount <= (width / frameWidth) * (height / frameHeight)`다. 모든 인덱스는 `0 <= index < frameCount`를 만족한다. 미사용 셀은 허용하되 참조하지 않는다.
- 절대 경로·상위 경로 탈출·루트 밖 symlink·원격 URL·실행 파일·HTML·스크립트는 프로젝트 자산으로 허용하지 않는다.
- 경로는 정규화 후 canonical root 내부 여부를 확인한다. 지원하지 않는 대소문자 경로 충돌과 Windows 예약 이름도 사전에 검출한다.
- 이미지 헤더와 한도를 먼저 검사한 뒤 디코딩한다. 파일 확장자만 신뢰하지 않는다.
- 알 수 없는 키와 잘못된 참조는 파일·필드 경로를 포함한 검증 오류다.
- 임의 `tauri.conf` 병합, Cargo 의존성, 빌드 스크립트·명령 삽입 필드는 제공하지 않는다.

## 6. 버전과 기본값

`schemaVersion`은 데이터 계약, Runtime/Builder 버전은 구현 릴리스, `product.version`은 생성 앱 버전이다. 세 버전을 구분해 빌드 보고서에 기록한다. 실행 앱에는 정확한 Engine 버전과 프로젝트 content hash를 포함한다.

Studio가 미래 버전 파일을 읽으면 원본을 보존하고 편집·빌드를 차단한다. 마이그레이션은 백업 또는 새 파일을 만들고 원자적으로 저장한다. JSON Schema와 Rust 타입을 같은 릴리스에서 관리하고, 문서 예시를 검증 fixture로 사용한다.

Rust 타입을 기준으로 JSON Schema·TypeScript DTO를 생성하고 CI에서 재생성 차이를 검사한다. 이미지 크기·파일 경로·참조 간 관계는 Rust 의미 검증기의 책임이다. 문서 예시 JSON의 구문 검사와 실제 자산 fixture의 통합 검증을 구분한다.

위치·배율·입력 토글은 사용자별 preferences로 덮어쓸 수 있다. 제품 이름·식별자·리소스·bindings는 생성 앱에서 편집하지 않고 제작자가 새 버전을 빌드한다. 새 기본값이 기존 사용자의 위치·크기 설정을 강제로 덮어쓰지 않는다.

preferences v1은 자체 schemaVersion=1과 scale·alwaysOnTop·basePlacement·input.keyboard/mouse·movementEnabled를 저장한다. 초기에는 프로젝트 기본값을 사용하고 사용자가 확정한 필드는 당시 기본값과 같더라도 명시적 override로 저장한다. 이후 프로젝트 기본값보다 그 값이 우선한다. basePlacement는 모니터 식별 힌트와 작업 영역 왼쪽 위 (0, 0) 기준의 로컬 DIP 좌표다. 자동 이동 좌표·편집 중 미확정 값·숨김·권한·실제 감지 상태·held·쿨다운·속도 집계·키워드 접두 상태는 저장하지 않는다. 재시작하면 표시 모드와 저장된 기본 배치로 시작하며 권한을 재점검한다. 저장·손상·초기화 절차는 [아키텍처](architecture.md)의 데이터 수명 계약을 따른다.

## 7. 추가 이미지·GIF·영역·타이밍 (호환 확장)

기존 image/sheet/frames는 그대로다. image는 기본 PNG 스프라이트와 표시 크기의 기준이며 resources는 manifest 기준 상대 경로다. 리소스 ID는 영문·숫자·`-`·`_` 1~64자, 최대 32개이며 `sheet`는 예약한다.

```json
{
  "resources": {
    "happy": { "image": "resources/happy.gif" },
    "photo": { "image": "resources/portrait.webp" }
  },
  "animations": {
    "idle": { "frames": [0], "fps": 2, "loop": true },
    "celebrate": { "resource": "happy", "useSourceTiming": true, "fps": 12, "loop": false },
    "dragging": { "resource": "photo", "fps": 12, "loop": true },
    "spriteDetail": { "regions": [{ "x": 256, "y": 0, "width": 128, "height": 256 }], "fps": 12, "loop": false }
  }
}
```

이는 manifest의 부분 예시이며 기존 schemaVersion/id/name/license/image/sheet/anchor/defaultAnimation과 함께 사용한다. frames와 regions는 동시에 채우지 않는다. regions는 실제 정적 이미지 안의 양수 너비·높이 사각형이며 최대 256개다. resource를 생략하면 기본 PNG를 자른다. resource와 frames를 함께 쓰면 그 원본의 프레임 번호를 선택한다. 정적 추가 이미지는 0번 한 프레임이다. GIF의 전체 프레임을 쓰려면 frames를 생략한다. GIF의 regions는 거부한다. useSourceTiming은 기본 true, false이면 fps(기본 12, 1~30)를 적용한다. GIF 프레임 시간 합계가 일회성 종료 시점이며 loop는 GIF 파일의 반복 메타데이터보다 우선한다. 표시 비율을 유지해 기준 프레임 안에 맞춘다. 선택적 `scalePercent`(정수 50~200, 기본 100)로 자동 맞춤 결과에 동작별 배율을 적용한다.

동작의 `sourceFacing`은 원본이 보는 방향이며 `left` 또는 `right`를 사용한다. 생략하면 오른쪽을 기준으로 자동 반전을 판단한다. 선택적 `anchorX`(0~1)는 프레임 왼쪽에서 잰 가로 기준점이다. 생략하면 캐릭터의 `anchor.x`를 사용한다. 반전할 때 기준점은 화면에서 같은 위치에 놓인다. Player 창은 모든 동작의 기준점과 배율을 고려해 좌우 공간을 확보한다.

`defaults.timing`은 `{ "rateWindowMs": 2000, "rateEvaluationMs": 50, "rateStabilityMs": 100, "idleTimeoutMs": 250 }`가 기본이다. 순서대로 범위는 100~10000, 10~250, 0~2000, 50~5000ms이고 평가 간격은 유휴 시간 이하여야 한다. 앱 사용자 설정은 프로젝트 기본값을 덮어쓰며 프로젝트 원본을 변경하지 않는다.

playAnimation의 `durationMs`는 선택적 1~60000ms 표시 시간이다. 생략하면 프레임 시간 합계, 지정하면 마지막 프레임을 유지한 채 그 시간까지 보여준다. `stopOnRelease`는 기본 false이며 `keyDown`, `mouseDown`, 버튼 누름만 포함한 `mouseInput`에서 설정할 수 있다. 반응을 시작한 키나 버튼을 떼면 재생을 즉시 중단한다. `drag` → `sustainAnimation`, `whileKeyDown` → `sustainAnimation`도 허용하며 반복 동작을 참조한다. 드래그가 최우선이고 놓으면 현재 기본 상태로 돌아간다.

## 타이머

선택 필드 `timer`의 `enabled`로 Player 타이머를 켠다. `mode`는 처음 사용할 `countdown` 또는 `pomodoro`다. `countdownSeconds`는 1~10800초(기본 600)다. `pomodoro`에서 `focusMinutes`는 1~120분(기본 25), `shortBreakMinutes`와 `longBreakMinutes`는 각각 1~60분(기본 5, 15), `rounds`는 2~8회(기본 4)다. Player의 개인 설정과 자주 쓰는 시간은 앱별 로컬 저장소에 기록한다.

`countdownPresets`는 표시 순서대로 서로 다른 1~10800초 값을 최대 10개 지정한다. 생략하면 1·5·10·25분 바로가기를 사용하며, 빈 배열이면 바로가기 없이 시작한다. Player에서 목록을 편집하면 개인 목록을 우선 사용한다. **Studio 기본 목록 복원**은 개인 목록을 지운 뒤 현재 배포 설정을 다시 읽는다.

`timerStarted`는 새 단계가 실행될 때, `timerCompleted`는 자연 만료 시에 발생한다. `phase`와 선택 필드 `alternatives`에는 서로 다른 `countdown`, `focus`, `shortBreak`, `longBreak`를 지정한다. 기존 완료 조건의 `break`는 두 휴식에 모두 대응한다. 재개는 남은 시간을 이어서 세며, 건너뛰어 다음 단계가 바로 시작되면 새 단계의 시작 반응이 발생한다. 여러 단계에 연결한 이벤트는 재반응 대기, 모션 선택 순서와 사운드를 공유한다. 입력·드래그·이동에 양보하며 최대 3초 대기한다. 숨김·잠금 중에는 반응 재생 없이 타이머 상태를 갱신한다.

`idleGroups`의 각 묶음에는 고유한 `id`, 선택적인 `name`, 하나 이상의 `states`를 둔다. 상태는 타이머 단계와 같은 값이며 프로젝트 전체에서 한 번만 선택할 수 있다. 선택적인 `animation`은 반복 동작을 지정한다. `cursorMode`는 기본 대기의 방향 설정을 쓰는 `inherit`, 묶음의 `directions`를 쓰는 `custom`, 방향 반응을 끄는 `off`다. 방향에 리소스가 없으면 묶음의 기본 모습, 이어서 필수 `defaultAnimation`을 사용한다. 타이머가 실행 중이며 다른 입력 동작을 재생하지 않을 때 묶음이 적용된다.

`buddyLongPress`는 버디 위에서 이동 기준을 넘지 않고 기본 버튼을 `holdMs`(300~2000ms) 이상 누른 뒤 놓을 때 발동한다. 프로젝트당 하나를 설정할 수 있다.
