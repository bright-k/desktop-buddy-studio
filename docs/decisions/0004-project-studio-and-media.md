# ADR 0004: 로컬 웹 Studio, 다중 이미지 리소스와 직접 드래그

상태: 채택 · 2026-11-07

사용자 요청에 따라 ADR 0002의 Studio 후속 일정, ADR 0003의 고정 속도 타이밍, 전체 클릭 통과·별도 위치 편집 결정을 아래 내용으로 대체한다. 기존 PNG 프로젝트 JSON은 계속 읽는다.

1. Studio는 Node 로컬 서비스 + TypeScript/Vite 웹 화면이다. `npm run studio` 또는 `Studio.command`로 실행하며 브라우저에서 프로젝트 생성·복제·보관·복원, 리소스·동작·이벤트·타이밍 편집, 검증·미리보기·빌드·산출물 다운로드를 제공한다. Player는 기존 Tauri 앱이다.
2. 프로젝트별 원본, 저장 버전, 미리보기 캐시, 작업 로그와 빌드 패키지를 `.data/projects/<packageId>`에 모은다. 저장은 불변 revision을 만든 뒤 현재 포인터를 원자적으로 교체한다. 빌드는 선택 revision을 사용하므로 편집과 분리된다. 낡은 revision 저장은 409 충돌로 거부한다. 영구 삭제 대신 보관/복원을 제공한다.
3. 캐릭터 `resources`는 PNG/JPEG/GIF/정적 WebP/BMP 원본을 참조한다. animation은 리소스, GIF 프레임 번호 또는 정적 이미지의 픽셀 사각형 배열을 선택한다. 기존 `image`·`sheet`·`frames`는 그대로 지원한다. 아이콘은 정사각형 PNG를 유지한다. SVG와 애니메이션 WebP/APNG는 지원하지 않는다.
4. Rust Project Core가 GIF의 합성 프레임과 지연을 디코딩하고 결정적 PNG render plan을 만든다. Canvas와 Rust Runtime이 같은 프레임 시간을 사용한다. GIF를 브라우저 Image 자체의 재생 시계에 맡기지 않는다. GIF 0ms 지연은 100ms, 33.33ms보다 짧은 지연은 30 FPS 한도에 맞춘다. 반복 여부는 animation.loop가 결정한다. [image GIF decoder](https://docs.rs/image/latest/image/codecs/gif/index.html)
5. `defaults.timing`은 속도 창 2000ms, 평가 50ms, 안정 100ms, 중단 250ms가 기본이다. Studio에서 모두 조절하고, 설치된 앱의 설정에서도 사용자 타이밍으로 덮어쓸 수 있다. 일회성 이벤트는 durationMs와 stopOnRelease를 추가한다.
6. 버디의 창 영역은 즉시 드래그를 받는다. 별도 편집 진입·완료/취소 메뉴와 휠 크기 변경을 제거한다. 놓을 때 위치를 저장하고 크기는 설정에서 조절한다. 전체 창 클릭 통과와 직접 드래그는 동시에 적용하지 않는다. 현재 드래그 범위는 선택된 모니터 작업 영역이며 다른 모니터 선택은 설정에서 한다.
7. drag → sustainAnimation을 정의할 수 있다. 드래그 동안 전역 입력 반응을 중지하고 drag 동작을 우선 표시한다. whileKeyDown → sustainAnimation도 지원한다. 우선순위는 drag > 일회성 > 키 유지 동작 > 속도 > 이동 > idle다. 동일 종류는 문서 순서를 따른다.
8. 트레이는 표시·일시 정지·키보드·마우스·이동을 체크 표시로 보여준다. 별도 상태 행에는 실제 감지 중/꺼짐/권한 필요/보류/실패를 표시한다. 설정 창은 표시·복원·포커스와 항상 위 옵션을 적용한다.
9. 로컬 서비스는 127.0.0.1에만 바인딩하고 Host/Origin 검사, 실행별 토큰과 HttpOnly SameSite 쿠키를 사용한다. 웹 UI는 외부에 호스팅하지 않는다. 명시적으로 연결한 GitHub Actions에만 고정 엔진·저장 버전 업로드를 제공한다. 웹 Studio에서 OS 전역 입력을 수집하지 않으며 테스트 입력은 Rust simulate로 전달한다.
10. 현재 장치는 macOS 14이므로 개발용 앱·ZIP 경로를 검증한다. macOS 15 이상 / Windows 11 25H2 이상에 대한 정식 배포·실기 검증은 별도 상태를 유지한다.

추가 확정: packageId=product.identifier를 프로젝트 폴더·API·브라우저의 공통 키로 사용한다. 기존 UUID 관리 키는 별칭으로 보존하며 앱의 내부 projectId는 유지한다. 저장된 revision과 별도로 충돌 검사를 하는 임시 편집을 자동 보관한다. 세부 동작과 검증은 [Studio 사용 안내](../studio.md)·[UX 검토](../studio-ux-review.md)를 따른다.

플랫폼 빌드는 현재 호스트 또는 GitHub Actions에서 수행한다. Studio의 원격 저장소 연결·업로드·완료 결과 동기화를 도입한다. 두 플랫폼의 앱은 같은 제품 식별자·구성 hash·고정 엔진을 사용한다. 기본 브랜치에는 워크플로 하나를 준비하고 엔진·프로젝트는 별도 브랜치에 저장한다. hosted runner 결과는 개발용으로 표시하며 실제 지원 OS의 입력·설치 검증과 구분한다. 인증과 무결성·복구 계약은 [Actions 연동 안내](../studio-actions.md)를 따른다.
