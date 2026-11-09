# Xem process manager hoạt động

## 1. Demo không cần model hay API key

Từ root repository:

```bash
cargo run --example process_sessions
```

Demo chạy một worker do chính binary ví dụ cung cấp, chỉ in tick, không sửa file
hay gọi model. Nó dùng scope backend giống desktop; đây không phải thêm background
vào CLI `ask` thông thường. Các bước mong đợi:

```text
1. First runtime ended; background <id> is still running: true
2. Independent UI reader sees: tick ...
3. Second runtime inspected same session; resources: 1
4. Another chat cannot read this session: PASS
5. Explicit stop: stopped=true, running=false
PASS: process reused across runtimes, isolated by chat, and cleaned up.
```

Điểm cần nhìn: hai runtime dùng cùng session, số resource vẫn là 1; reader UI không
làm agent mất log; scope khác bị từ chối; Stop dọn worker. Demo tự cleanup ở cuối.

## 2. Thử trong Tauri với model đang cấu hình

Khởi động lại phiên Tauri dev cũ để backend nạp các command mới, rồi chạy từ root:

```bash
npm --prefix desktop run tauri dev
```

Mở một chat, chọn folder rồi gửi yêu cầu tường minh, ví dụ trên Linux/WSL có Python:

> Chạy lệnh này như background trong chat này, không thêm `&`, giữ qua các tin nhắn:
> `python3 -u -c "import time; [(print('tick', i, flush=True), time.sleep(1)) for i in range(600)]"`.
> Sau khi khởi động, trả lời mã process session. Không khởi động thêm bản khác.

Chọn Allow nếu đang dùng Ask/Workspace. Mở disclosure **Processes** phía trên
composer: thấy một dòng background, trạng thái Running và cwd. Mở **View output**
để thấy tick tăng. UI xem log bằng cursor riêng.

Gửi tin nhắn tiếp theo:

> Liệt kê process của chat và đọc output mới của session vừa tạo. Không chạy lại lệnh.

Session ID và số process không đổi. Đổi sang chat khác: không nhìn thấy process
của chat đầu. Quay lại chat đầu: process vẫn chạy. Click **Stop**, chờ Stopped;
log ngừng tăng. Stop không rollback tác động trước đó.

Model có thể chọn tool chưa đúng; kiểm tra Activity của câu trả lời để xác nhận nó
đã dùng `lifetime: background`, thay vì chỉ dựa vào lời nói. Nếu nó chạy job mặc
định thì resource sẽ bị dọn cuối lượt. `list_processes`/`read_process` là thao tác
đọc; model không cần chạy lại command để kiểm tra.

Với Windows native, dùng lệnh foreground tương đương trên máy của bạn. Python là
phụ thuộc của ví dụ prompt này, không phải phụ thuộc runtime Atlas; demo Rust ở
mục 1 không cần Python. Test native Windows có thể bị Application Control chặn;
không tắt policy để thử và không suy ra runtime acceptance từ compile.

## 3. Phân biệt job

Trong chat khác, yêu cầu chạy một lệnh dài với `lifetime: job`, đọc một phần output
rồi kết thúc lượt. Job bị dọn khi lượt kết thúc, khác với background ở mục 2.
Không hứa dev server của một job sẽ sống sau final answer.

## 4. Nếu không thấy process

- Mở Processes và dùng **Refresh processes**; lỗi/retry hiện tại chỗ.
- Command có thể đã kết thúc hoặc lỗi: xem status, exit code và output.
- Record cũ có thể đã bị eviction theo giới hạn; mất ID không tự rerun command.
- New chat/chuyển chat release scope trống cũ. Failed draft có message/background
  vẫn có owner để quay lại Stop; menu **Conversation actions → Close draft** dọn
  process và đóng draft mà không xóa saved history. Reload dọn draft client cũ.
- Xóa saved conversation dọn resource của nó; shutdown app dọn manager. Không tự
  phục hồi process sau restart, không ghi conversation chỉ vì mở Processes.

Chi tiết: [Process sessions](../architecture/process-sessions.md).
