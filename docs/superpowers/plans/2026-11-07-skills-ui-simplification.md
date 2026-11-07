# Skills UI simplification

User-requested correction: current Skills has too many simultaneous controls and raw warnings. Implement progressive disclosure using the existing source/import APIs and scope ownership. No new backend behavior or symlink permission changes.

- [x] Tests first: default view has one Add skill action, scope/search, compact cards with switch and Details. Import UI stays hidden until requested; select GitHub/folder/file to show only the matching controls. Raw catalog warnings are in a closed disclosure. Preserve editors/details/focus across updates and all existing import context tests.
- [x] Implement a top Add skill toggle, one source picker, compact scoped library, real accessible switches, per-card Details and Library options for refresh/installer instructions. Preserve mounted import fields/previews and in-flight state when closing/reopening.
- [x] Keep issue diagnostics intact in one collapsed group with useful symlink recovery guidance. Fatal list/action errors stay actionable.
- [x] Run focused and full frontend tests/build, a real-browser screenshot check of the initial view and add flow, and Tauri debug build. Independent read-only review after implementation.

Existing Skills installation/runtime changes in the shared checkout belong to the preceding authorized task and must be preserved.

## Verification and review

- Initial RED: four missing simplified UI behaviors; GREEN after progressive-disclosure implementation.
- Review RED: focus loss during switch save, late reads reopening Details, and folder previews visible under file source all reproduced; fixes and regressions added. Enter handling separately tested RED then GREEN to avoid submitting Settings.
- Full desktop suite: 281 tests passed (38 Skills tests). TypeScript/Vite production build and Tauri debug/no-bundle build exit 0. git diff --check passed.
- Real Chromium render of the production Skills component using fixture API data: initial view has exactly one visible button; raw diagnostics collapsed, keyboard Space/Enter retain switch focus, selected sources hide other controls, closing/reopening retains drafts, and no horizontal overflow at 390px. Screenshots in /tmp/atlas-skills-visual (library.png, add.png, mobile.png). This is a component browser check, not a live provider/native-dialog test.
- Independent read-only review confirmed all actionable findings resolved; 38 focused tests passed in the reviewer's run. Backend symlink protections and all previous runtime work preserved. User subsequently requested a local feature commit; no publication.
