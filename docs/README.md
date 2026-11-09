# Tài liệu Atlas

Thư mục này chứa tài liệu tham chiếu về tính năng đã triển khai và các thiết kế
được user duyệt rõ ràng. Mỗi thiết kế phải ghi tình trạng triển khai; được duyệt
không đồng nghĩa với đã có trong sản phẩm.

| Tài liệu | Trạng thái |
| --- | --- |
| [Process sessions](architecture/process-sessions.md) | Tham chiếu hành vi hiện tại, 2026-11-09 |
| [UniKey trên WSLg](platforms/wslg-unikey.md) | Ghi nhận tái hiện và cách khắc phục đã kiểm chứng |

## Draft và quy trình duyệt

- Draft thiết kế: `.superpowers/specs/`.
- Plan, ledger, log và preview: các thư mục tương ứng trong `.superpowers/`.
- Những nội dung này chỉ lưu local, được Git bỏ qua, không có trong bản clone mới.
- User yêu cầu viết tài liệu là cho phép soạn draft; không tự coi đó là duyệt thiết kế.
- Khi user duyệt, chuyển bản đã chốt vào phần phù hợp trong `docs/`, ghi trạng thái,
  sửa link và cập nhật bảng tài liệu phía trên. Chỉ commit bản đã chốt, giữ plan/log local.
- Không dùng `git add -f` để đưa draft hoặc artifact Superpowers vào repository.

Các workflow document trước đây được lưu nguyên nội dung ở archive local
`.superpowers/archive/2026-11-09-docs/`, kèm manifest SHA-256. Phiên bản cũ cũng còn
trong Git history. Archive là bản lưu lịch sử, không phải danh sách thiết kế đã duyệt.
