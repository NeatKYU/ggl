## 설치

1. 아래 **Assets**에서 `ggl-…-macos-universal.zip`을 받아 압축을 풉니다. Apple Silicon과 Intel Mac 모두 지원해요.
2. `ggl.app`을 `응용 프로그램` 폴더로 옮깁니다.
3. 처음 열 때 macOS가 "Apple에서 확인할 수 없음"이라고 막으면:
   - `시스템 설정 → 개인정보 보호 및 보안`으로 가서 아래쪽의 **그래도 열기**를 누르거나,
   - 터미널에서 `xattr -dr com.apple.quarantine /Applications/ggl.app`을 실행하세요.

   Apple 개발자 서명이 없는 앱이라서 한 번만 필요한 과정이에요.
