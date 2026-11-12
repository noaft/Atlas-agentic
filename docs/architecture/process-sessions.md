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
| `read_process` | `session_id`, tùy chọn `wait_ms`, `cursor` hoặc `tail_lines` | ReadOnly |
| `stop_process` | `session_id` | Command |

Ask/Workspace xác nhận start và tool Stop; Full access giữ semantics bỏ qua xác
nhận Atlas, không nâng quyền OS. Native API Stop vẫn kiểm tra ownership scope;
UI hiện dùng tool `stop_process` qua agent, không có nút Stop riêng.

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
chỉ lấy phần mới, giữ cursor qua các lượt của chat. Native API hỗ trợ cursor riêng cho reader độc lập. UI hiện tại chỉ
hiển thị kết quả tool inline, không có viewer polling riêng. Cursor tùy chọn có dạng:

```json
{"stdout":0,"stderr":0}
```

Để xem những dòng cuối, gọi:

```json
{"session_id":"<id đã nhận>","tail_lines":50,"wait_ms":0}
```

`tail_lines` nhận 1–1000, lấy tối đa N dòng cuối **riêng cho stdout và stderr**,
không ghép thứ tự hai luồng. Không kết hợp với `cursor`; không làm dịch cursor
mặc định hay cursor UI. Giữ `wait_ms` 0–10000: có log đọc được thì trả ngay;
chưa có thì chờ output hoặc kết thúc trong deadline. Một dòng kết thúc bằng LF;
LF cuối không tạo thêm dòng rỗng, dòng chưa có LF vẫn được tính, CRLF giữ nguyên.
Tail chỉ lấy phần còn trong ring 64 KiB mỗi luồng, có thể bắt đầu giữa dòng.

Mỗi kết quả có `stdout_view` / `stderr_view`: `start_byte`, `end_byte` (exclusive),
`total_bytes`, `start_line` / `end_line` (null khi không có text), `total_lines`,
`returned_lines`, `lines_after`, `starts_mid_line`, `older_output_dropped`, `has_more`.
Số dòng tuyệt đối là 1-based, vẫn tính các LF đã bị bỏ khỏi buffer. `has_more` báo
byte chưa trả tại snapshot (ví dụ UTF-8 chưa đủ byte), không dự đoán output tương lai.
`older_output_dropped` phân biệt mất log do ring overflow với chủ động chọn tail;
`stdout_truncated` / `stderr_truncated` vẫn chỉ báo gap của cursor được yêu cầu.
Metadata phản ánh lúc đọc, không phải phần trăm hoàn thành của công việc.

Terminal và Activity hiển thị khoảng dòng/byte, tail và các cảnh báo trên. Khi
phần output quá dài cho ngân sách Activity, giữ JSON hợp lệ và metadata, rút
ngắn phần text về cuối rồi ghi `display_truncated`; khoảng dòng mô tả toàn bộ
output đã chụp, không chỉ preview. Khi hết ngân sách cả turn, details vẫn có thể
bị bỏ/cắt như trước. Không có polling hoặc panel process riêng.

Đọc explicit cursor không thay cursor mặc định. Kết quả trả `next_cursor` để lần
sau tiếp tục và `oldest_cursor` để nhận biết phần còn giữ. Nếu cursor cũ bị buffer
vượt qua, `stdout_truncated`/`stderr_truncated` báo mất dữ liệu. Đọc log không chạy
lại lệnh. Một lần đọc có thể không có output mà process vẫn đang chạy.

| Trường kết quả | Ý nghĩa |
| --- | --- |
| `session_id`, `running` | Resource được đọc và worker còn đang chạy/cleanup hay không. |
| `stdout`, `stderr` | Phần output từ cursor của reader, giới hạn theo buffer byte. |
| `next_cursor`, `oldest_cursor` | Offset tiếp theo và offset cũ nhất còn giữ. |
| `tail_lines` | Số dòng tail được yêu cầu; null với read thông thường. |
| `stdout_view`, `stderr_view` | Vị trí output và tình trạng retention tại thời điểm đọc. |
| `stdout_truncated`, `stderr_truncated` | Có gap; không được báo log là đầy đủ. |
| `exit_code` | Mã thoát nếu có; `null` không chứng minh thành công. |
| `timed_out`, `stopped` | Worker đã xác nhận timeout hoặc dừng. |
| `stop_requested` | Mới gửi yêu cầu stop; khác với đã dừng. |
| `error` | Lỗi worker/khởi chạy nếu có. |

Stop có thể trả `running: true` khi cleanup chưa xác nhận; lúc đó
agent cần đọc lại trạng thái. Dừng/timeout không hoàn tác tác động command đã thực hiện.
UTF-8 bị chia giữa các lần đọc được giữ tới khi đủ byte; byte không hợp lệ được
biểu diễn thay thế. Terminal inline hiển thị dữ liệu tool event trả về trong vùng
cuộn; Activity lưu bản bounded theo chính sách hiện có. Không có output streaming
riêng từ process manager, không có log archive replay vô hạn.

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
  [UI](../../desktop/src/chat-view.ts), [API adapter](../../desktop/src/atlas-api.ts).
- [Demo local](../../examples/process_sessions.rs) và
  [test scope/process](../../tests/chat_process_behavior.rs).
