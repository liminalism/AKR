# Embedded fonts

`NotoSans-Regular.ttf` is Noto Sans, © Google, licensed under the
SIL Open Font License, Version 1.1
(https://openfontlicense.org/open-font-license-official-text/).
It is embedded in the `akr-gui` binary (`include_bytes!`) rather than loaded
from the system, so the review workbench renders identical type on Windows
and Linux with no installed-font dependency. Unmodified upstream release.
