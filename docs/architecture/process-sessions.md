# Process sessions trong Atlas

Trạng thái: triển khai desktop manager theo chat, ngày 2026-11-09. Quyết định và
ownership được mô tả trong [Tool/resource lifecycle](tool-resource-lifecycle.md).
Hướng dẫn thử: [Xem process manager hoạt động](../guides/process-manager-demo.md).

## Ba khái niệm khác nhau

- **Chat session** là cuộc hội thoại/frontend draft.
- **Process session** là một lần chạy lệnh, có thể gồm shell và nhiều process OS con.
- **Runtime request** là một lượt agent, gồm nhiều model/tool calls.

OS quản lý process thật. Atlas giữ handle, scope chủ sở hữu, trạng thái, timeout
và output có giới hạn; model chỉ dùng UUID `session_id`, không điều khiển bằng PID.

Desktop giữ `ProcessManager` cùng backend app, mỗi chat có scope riêng. Runtime
mới nhận scope handle của chat hiện hành. CLI vẫn chỉ giữ resource trong một lượt;
chưa có daemon hoặc terminal tồn tại qua nhiều lần chạy CLI.

## Job và background

| Thuộc tính | Ý nghĩa |
| --- | --- |
| `kind: process` | Loại tài nguyên, hiện có process. |
| `lifetime: job` | Mặc định; dọn khi lượt hoàn tất, lỗi hoặc bị hủy. |
| `lifetime: background` | Chỉ dùng khi user yêu cầu rõ ràng; desktop giữ qua nhiều lượt trong cùng chat. |

Background vẫn có timeout; không phải dịch vụ vô hạn. Chạy chương trình foreground,
không dùng `&` để tách nó khỏi vòng quản lý. Khi shell thoát, các tiến trình con còn
trong process group/Windows Job sẽ được dọn, kể cả khi đã redirect output.

Chat nháp được cấp scope trong bộ nhớ, không tạo history chỉ để giữ scope. Lần
trả lời đầu lưu thành công sẽ gắn scope với conversation mà giữ nguyên session ID.
Failed draft có background vẫn xem/Stop được; New chat không lấy resource của nó.
Menu Close draft có thể dọn scope và bỏ draft; không tạo/xóa saved history.
Empty draft bị bỏ khi New chat/chuyển chat được release. Reload webview dọn draft scope của
client cũ; saved chat có thể gắn lại background trong backend còn sống.

## API tool

| Tool | Tham số chính | Quyền |
| --- | --- | --- |
| `exec_command` | `command`, tùy chọn `yield_time_ms`, `timeout_ms`, `lifetime` | Command |
| `list_processes` | Object rỗng | ReadOnly |
| `read_process` | `session_id`, tùy chọn `wait_ms`, `cursor` | ReadOnly |
| `stop_process` | `session_id` | Command |

Ask/Workspace xác nhận start và tool Stop; Full access giữ semantics bỏ qua xác
nhận Atlas, không nâng quyền OS. Stop từ UI là thao tác user tường minh trên scope
đã được backend resolve, không phải quyền model chạy thêm một command mới.

Schema/catalog có metadata trạng thái ngắn của scope để agent nhận ra resource
đã có. Không preload command hoặc raw stdout/stderr. Agent nên dùng danh sách để
kiểm tra cwd/trạng thái trước khi tái sử dụng hoặc tạo thêm server; không tự replay
hay restart command thất bại.

Desktop quản lý cả command đồng bộ: không có `yield_time_ms` thì chờ kết quả như
trước, nhưng process vẫn xuất hiện trong manager và tính vào giới hạn. Có yield
thì trả session khi chưa hoàn tất. Background không truyền yield sẽ mặc định yield
1000 ms. CLI đồng bộ giữ hành vi cũ; CLI từ chối background.

Ví dụ arguments (không phải toàn bộ envelope giao thức):

```json
{"command":"cargo test","yield_time_ms":1000,"lifetime":"job"}
```

```json
{"command":"npm run dev","yield_time_ms":1000,"lifetime":"background"}
```

```json
{"session_id":"<mã tool trả về>","wait_ms":1000}
```

```json
{"session_id":"<mã tool trả về>"}
```

Hai ví dụ cuối tương ứng `read_process` và `stop_process`. Dùng đúng mã đã trả về;
không đoán PID/session của chat khác. Lỗi scope/ID không tự khởi động lại lệnh.

## Output và cursor

Buffer stdout/stderr là ring có offset tăng tuyệt đối. Reader mặc định của agent
chỉ lấy phần mới, giữ cursor qua các lượt của chat. UI dùng cursor riêng, nên mở
log không lấy mất output của agent. Cursor tùy chọn có dạng:

```json
{"stdout":0,"stderr":0}
```

Đọc explicit cursor không thay cursor mặc định. Kết quả trả `next_cursor` để lần
sau tiếp tục và `oldest_cursor` để nhận biết phần còn giữ. Nếu cursor cũ bị buffer
vượt qua, `stdout_truncated`/`stderr_truncated` báo mất dữ liệu. Đọc log không chạy
lại lệnh. Một lần đọc có thể không có output mà process vẫn đang chạy.

| Trường kết quả | Ý nghĩa |
| --- | --- |
| `session_id`, `running` | Resource được đọc và worker còn đang chạy/cleanup hay không. |
| `stdout`, `stderr` | Phần output từ cursor của reader, giới hạn theo buffer byte. |
| `next_cursor`, `oldest_cursor` | Offset tiếp theo và offset cũ nhất còn giữ. |
| `stdout_truncated`, `stderr_truncated` | Có gap; không được báo log là đầy đủ. |
| `exit_code` | Mã thoát nếu có; `null` không chứng minh thành công. |
| `timed_out`, `stopped` | Worker đã xác nhận timeout hoặc dừng. |
| `stop_requested` | Mới gửi yêu cầu stop; khác với đã dừng. |
| `error` | Lỗi worker/khởi chạy nếu có. |

UI giữ Stopping khi cleanup chưa xác nhận. Stop có thể trả `running: true`, lúc đó
đọc lại trạng thái. Dừng/timeout không hoàn tác tác động command đã thực hiện.
UTF-8 bị chia giữa các lần đọc được giữ tới khi đủ byte; byte không hợp lệ được
biểu diễn thay thế. UI giữ tối đa 64 KiB ký tự hiển thị mỗi viewer, độc lập với
buffer byte backend; đây không phải log archive có thể replay vô hạn.

## Giới hạn

| Hạng mục | Giá trị |
| --- | --- |
| Active / chat scope | 8 |
| Active / backend manager | 32 |
| Record / chat scope | 32 |
| Record / backend manager | 128 |
| Buffer / stdout hoặc stderr | 64 KiB byte; overflow bỏ phần cũ |
| Yield | 0–10000 ms |
| Read wait | 0–10000 ms; mặc định 1000 ms |
| Command đồng bộ mặc định | 30 giây |
| Job session | Mặc định/tối đa 10 phút |
| Background | Mặc định 1 giờ, tối đa 24 giờ |

Loại record hoàn tất cũ khi cần chỗ; ID có thể hết hạn dù process đã hoàn tất.
Giới hạn session không giới hạn tổng process OS con hay CPU/RAM của command.

## Cleanup và scope

Job chỉ bị dọn theo request sở hữu nó. Background giữ tới Stop, timeout, xóa chat,
đóng draft hoặc shutdown app bình thường; chuyển chat/Settings không dừng nó.
Process giữ cwd lúc spawn. Thay workspace/quyền không đổi cwd hoặc cấp approval
mới cho resource cũ; các thao tác sau vẫn dùng policy hiện hành.

Shutdown dọn trên worker với deadline, không join trên Tauri event thread. Unix
kill group trước khi reap leader, dùng WNOWAIT tránh PID reuse; Windows tạo process
suspended và chứa trong Job Object kill-on-close trước khi resume. Không bảo đảm
cleanup trước mọi kiểu crash/SIGKILL/mất điện, hoặc command chủ động thoát khỏi
containment. Không thêm frontend shell/filesystem/process permissions.

Không lưu process handle xuống đĩa, không khôi phục session hay tự chạy lại lệnh
sau restart. Không có stdin/PTY hoặc terminal nhận nhiều command nối tiếp.
Activity lưu thành công có thể chứa chi tiết đã sanitize/bounded, không phải một
process còn sống hoặc kho lưu toàn bộ log.

## Mã nguồn

- [Manager/scope](../../src/tools/process_sessions/manager.rs),
  [output/cursor](../../src/tools/process_sessions/output.rs),
  [tool handlers](../../src/tools/process_sessions.rs).
- [Native ownership/IPC](../../desktop/src-tauri/src/process_commands.rs),
  [UI](../../desktop/src/processes-panel.ts), [API adapter](../../desktop/src/atlas-api.ts).
- [Demo local](../../examples/process_sessions.rs) và
  [test scope/process](../../tests/chat_process_behavior.rs).
