# Vietnamese input preservation fix

User authorizes diagnosis/fix without further questions and a separate feature commit. Prior Skills work and per-feature commit policy were committed as 71a4451.

## Evidence and scope

The native composer is already stationary; draft input does not emit a full render. However its resize handler sets the live textarea height to zero on every input, including composition/replacement input. Keyboard handling only checks event flags, so Enter during a composition lifecycle with missing isComposing/keyCode flags can submit and clear a draft. replaceDraft writes even identical values. Remove these reproducible disruptions rather than implementing Telex conversion or rewriting Unicode text. Windows UniKey itself is unavailable in this Linux environment; do not claim native Windows verification from browser simulations.

- [x] RED: tests for no live height collapse during replacement/backspace input, no layout mutations during composition, lifecycle Enter/submit/task guards, and idempotent explicit draft replacement.
- [x] GREEN: measure auto-height in a hidden non-focusable textarea with mirrored typography/width; write live height only when needed, never collapse the live field. Track composition lifecycle; defer layout while composing. Leave input/beforeinput/backspace/paste native and preserve draft/caret.
- [x] Guard slash task interactions while composing; reset transient state only on intentional changed draft/session transitions. Preserve existing Enter/Shift+Enter behavior after composition.
- [x] Run full frontend tests/build, real Chromium IME/replacement checks using CDP, and Tauri debug build; independent code review.
- [x] Run cargo fmt --check and cargo test, inspect staged diff/status and create a focused local fix commit. Report the commit and native-test limitation.

## Results

- First RED run: five failures reproduced live height collapse/layout during composition, lifecycle Enter/task submission, and same-value writes. Initial fix passed the complete frontend suite.
- Review follow-up RED: omitted compositionend during blur and equal-text session changes could keep Send disabled. Added explicit blur/session cancellation guards, with both regressions now green.
- Final verification: all 288 frontend tests passed, TypeScript/Vite production build and Tauri debug/no-bundle build succeeded, cargo fmt --check and cargo test passed; staged diff checked. Independent read-only review found no remaining actionable issues.
- Chromium CDP: imeSetComposition through t/ti/tiế/tiếng followed by committed tiếng Việt preserved the draft through background updates; unflagged composing Enter did not submit; committed Enter submitted exact text. Native backspace/replacement and clipboard-style insertText preserved Vietnamese; raw Telex remained unchanged with conversion off. Measured height grew to288px and shrank to37px without zeroing the live field; hidden mirror cleaned up on destroy.
- Windows UniKey native acceptance remains unverified: this environment is Linux. Browser IME checks exercise the rendering/input pipeline but are not proof of Windows OS/UniKey compatibility. No native permission changes, dependencies, clipboard interception or JavaScript Telex conversion were added.
