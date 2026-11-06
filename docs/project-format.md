# 프로젝트와 캐릭터 리소스 형식

상태: v1 스키마 제안 / 기준일: 2026-11-07

이 문서의 JSON은 구현할 계약의 예시다. 예시 이미지·JSON Schema 파일·CLI는 아직 저장소에 존재하지 않는다.

## 1. 프로젝트 단위

프로젝트 하나가 독립 앱 하나를 정의한다. 여러 프로젝트를 같은 소스 트리에 두어도 된다. 앱별 사용자 설정은 프로젝트 원본이 아니라 설치된 앱의 데이터 디렉토리에 저장한다.

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
      "id": "typing",
      "trigger": { "type": "keyDown", "code": "*", "repeat": false },
      "effect": { "type": "playAnimation", "animation": "typing", "priority": 30, "retrigger": "restart", "cooldownMs": 120 }
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
      "id": "right",
      "trigger": { "type": "whileKeyDown", "code": "ArrowRight" },
      "effect": { "type": "move", "x": 1, "y": 0, "speedDipPerSecond": 120 }
    }
  ],
  "build": {
    "targets": ["windows-x64", "macos-arm64"],
    "signing": { "windows": "none", "macos": "ad-hoc" }
  }
}
```

`movementEnabled`가 false이면 이동 매핑은 저장하되 실행하지 않는다. 사용자가 설정에서 켤 수 있다. 감지된 Space는 jump가 typing보다 우선하고, 원래 앱에도 전달된다.

### 필드 계약

| 필드 | v1 규칙 |
| --- | --- |
| `schemaVersion` | 정수 1. 미래 버전은 명시적인 마이그레이션 없이는 빌드하지 않는다. |
| `projectId` | UUID. 편집기에서 프로젝트를 식별한다. 새 앱으로 복제하면 새 값이다. |
| `product.slug` | 소문자 영문·숫자·하이픈, 2~64자. 실행 파일·산출물 이름의 기반이다. |
| `product.name` | 표시 이름, 1~80자. 경로 구분자·제어 문자는 금지한다. |
| `product.version` | 최초에는 `숫자.숫자.숫자` 형식. 플랫폼 패키지 버전으로 변환 가능한 범위만 허용한다. |
| `product.identifier` | 영문·숫자로 시작하는 점 구분 reverse-DNS 형식. 제품마다 고유하게 유지한다. 예시는 예약된 example 이름이다. |
| `product.icon` | 프로젝트 루트 기준 PNG 경로. 빌더가 OS별 아이콘 파일을 생성한다. |
| `character` | 프로젝트 루트 기준 캐릭터 manifest 경로. |
| `build.targets` | 허용 목록 값만 받는다. 대상 지원 여부는 빌드 호스트 검사와 별개다. |
| `build.signing` | Windows `none` 또는 `self-signed`, macOS `ad-hoc` 또는 `self-signed`. 키 정보는 넣지 않는다. |

`product.identifier`는 같은 앱 업데이트에서 유지한다. 새 이름으로 복제한 앱에는 새 식별자를 만든다. 같은 식별자와 다른 프로젝트를 함께 빌드하면 충돌을 경고하고 독립 제품으로 배포하지 못하게 한다. 설정 파일만으로 시스템 전체의 식별자 유일성을 보장할 수는 없다.

필수 필드는 `schemaVersion`, `projectId`, `product`, `character`다. 나머지 생략값은 이 문서의 defaults와 동일한 기본값, bindings 빈 배열, 현재 호스트의 지원 대상, Windows none/macOS ad-hoc로 정규화한다. 명시한 필드에 잘못된 값이 있으면 기본값으로 조용히 대체하지 않는다.

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
    "typing": { "frames": [2, 3], "fps": 12, "loop": false },
    "click": { "frames": [4, 5], "fps": 12, "loop": false },
    "jump": { "frames": [6, 7, 8, 9], "fps": 12, "loop": false },
    "walk": { "frames": [10, 11, 12, 13], "fps": 8, "loop": true }
  }
}
```

프레임은 왼쪽 위부터 행 우선, 0부터 시작한다. 각 프레임 크기와 anchor를 동일하게 유지해 동작 전환 시 캐릭터가 튀지 않도록 한다. `idle`은 필수이며 defaultAnimation과 일치해야 한다. 나머지 이름은 프로젝트 bindings가 참조할 때만 필수다. FPS는 1~30, anchor는 각 축 0~1이다.

manifest의 이미지·라이선스 경로는 manifest가 있는 디렉토리 기준이다. 프로젝트의 다른 자산 경로 기준과 혼동하지 않는다. 이름만 적은 라이선스를 권리 증명으로 해석하지 않으며 제작자가 사용 권한을 확보한 리소스를 포함한다.

## 4. 이벤트·행동 계약

| Trigger type | 허용 effect | 의미 |
| --- | --- | --- |
| `keyDown`, `keyUp` | `playAnimation` | 물리 키 ID, `*`는 매핑 가능한 일반 키 전체. repeat 옵션 적용 |
| `whileKeyDown` | `move` | 해당 키의 held 상태에서 이동. `*`는 허용하지 않는다. |
| `mouseDown`, `mouseUp` | `playAnimation` | `primary`, `secondary`, `middle` 버튼 |
| `wheel` | `playAnimation` | `up`, `down`, `left`, `right` 방향. OS 델타는 정규화한다. |
| `cursorMove` | `faceCursor` | 캐릭터의 좌우 방향을 커서에 맞춘다. PNG 프레임은 수평 반전한다. |

`playAnimation.priority`는 0~100, `cooldownMs`는 0~10000, `retrigger`는 `restart`/`ignore`다. 낮은 우선순위의 animation은 높은 우선순위 재생 중 시작하지 않는다. 동시에 생긴 후보는 priority, 매핑 정의 순서로 결정한다. 움직임과 바라보기는 별도 축이며 animation과 함께 적용할 수 있다.

move의 x/y는 -1~1, 0 벡터는 금지하고 speed는 1~1000 DIP/초다. code는 Engine이 공개하는 정규 키 목록으로 제한하며 설정에서 알 수 없는 플랫폼 키를 임의 문자로 치환하지 않는다. 보조·멀티미디어 키의 전체 지원은 어댑터별 지원표로 명시한다.

## 5. 검증과 자산 한도

초기 한도는 제안값이며 구현·성능 측정 후 버전 정책에 맞춰 조정한다.

- JSON은 UTF-8, 프로젝트 JSON 최대 1 MiB, 총 매핑 최대 256개다.
- PNG만 지원한다. 파일 1개 16 MiB, 시트 한 변 4096 px, 디코딩 RGBA 64 MiB, 프로젝트 자산 합계 64 MiB 이하로 제한한다.
- 프레임 한 변 16~512 px, frameCount 1~256, 시트의 실제 이미지 크기와 선언값이 일치해야 한다.
- 시트 크기는 프레임 크기의 정수 배이며 모든 참조 인덱스가 실제 셀 범위 안에 있어야 한다.
- 절대 경로·상위 경로 탈출·루트 밖 symlink·원격 URL·실행 파일·HTML·스크립트는 프로젝트 자산으로 허용하지 않는다.
- 경로는 정규화 후 canonical root 내부 여부를 확인한다. 지원하지 않는 대소문자 경로 충돌과 Windows 예약 이름도 사전에 검출한다.
- 이미지 헤더와 한도를 먼저 검사한 뒤 디코딩한다. 파일 확장자만 신뢰하지 않는다.
- 알 수 없는 키와 잘못된 참조는 파일·필드 경로를 포함한 검증 오류다.
- 임의 `tauri.conf` 병합, Cargo 의존성, 빌드 스크립트·명령 삽입 필드는 제공하지 않는다.

## 6. 버전과 기본값

`schemaVersion`은 데이터 계약, Runtime/Builder 버전은 구현 릴리스, `product.version`은 생성 앱 버전이다. 세 버전을 구분해 빌드 보고서에 기록한다. 실행 앱에는 정확한 Engine 버전과 프로젝트 content hash를 포함한다.

Studio가 미래 버전 파일을 읽으면 원본을 보존하고 편집·빌드를 차단한다. 마이그레이션은 백업 또는 새 파일을 만들고 원자적으로 저장한다. JSON Schema와 Rust 타입을 같은 릴리스에서 관리하고, 문서 예시를 검증 fixture로 사용한다.

위치·배율·입력 토글은 사용자별 preferences로 덮어쓸 수 있다. 제품 이름·식별자·리소스·bindings는 생성 앱에서 편집하지 않고 제작자가 새 버전을 빌드한다. 새 기본값이 기존 사용자의 위치·크기 설정을 강제로 덮어쓰지 않는다.
