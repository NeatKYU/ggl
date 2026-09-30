## 설치

**터미널 한 줄로 설치 (추천).** Apple Silicon과 Intel Mac 모두 지원해요.

```sh
curl -fsSL https://raw.githubusercontent.com/NeatKYU/ggl/main/install.sh | sh
```

`/Applications/ggl.app`과 터미널 명령 `ggl-open`을 설치합니다. 이 방법은 macOS의 "확인되지 않은 개발자" 경고가 뜨지 않아요.

**디스크 이미지로 설치.**

1. 아래 **Assets**에서 `ggl-…-macos-universal.dmg`를 받아서 엽니다.
2. `ggl.app`을 옆의 `Applications` 폴더로 끌어다 놓습니다.
3. 처음 열 때 macOS가 "Apple에서 확인할 수 없음"이라고 막으면:
   - `시스템 설정 → 개인정보 보호 및 보안`으로 가서 아래쪽의 **그래도 열기**를 누르거나,
   - 터미널에서 `xattr -dr com.apple.quarantine /Applications/ggl.app`을 실행하세요.

   Apple 개발자 서명이 없는 앱이라서 한 번만 필요한 과정이에요.
