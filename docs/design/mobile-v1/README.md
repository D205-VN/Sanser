# Sanser Mobile — Concept 01

Bản xem trước UI/UX cho Android và iOS, ngày 31/10/2026. Đây là thiết kế đề xuất,
chưa phải ứng dụng mobile và chưa được tạo thành file Figma online.

- Xem `overview.png` để duyệt nhanh sáu màn chính.
- Mở `preview.html` trong trình duyệt để thử các luồng bằng vùng bấm.
- `android-preview.png` là biến thể màn danh sách máy Android.
- `screens/*.svg` chứa từng màn dưới dạng vector.
- `design.json` là nguồn dữ liệu lớp; `generate.py` tạo các artifact từ cùng một mô hình.

## Hướng thiết kế

Giữ nền tối và nhận diện tím nhạt/xanh ngọc của Sanser, thay sidebar bằng ba tab
Máy tính / Chia sẻ / Tài khoản. Nút kết nối chính dễ tìm, các trạng thái trực tuyến
đi cùng chữ, không phụ thuộc màu. Thông tin kết nối kỹ thuật không chen vào luồng
chọn máy. Chỉ hiện số đo sau khi có dữ liệu; màn demo đánh dấu rõ số minh họa.

Tách điều khiển và chỉ xem, có bước chờ hình đầu tiên và trạng thái không nhận
được dữ liệu. Phiên ngang giữ bảng công cụ nhỏ ở phía trên, có truy cập bàn phím,
cử chỉ và rời phiên. Hướng dẫn chạm là đề xuất, cần thử trên máy thật trước khi chốt.

Voice thuộc phiên chia sẻ: mic mặc định tắt, có người đang nói, tắt mic và tắt loa
độc lập. Phần mở rộng dùng bottom sheet; thanh công cụ thu gọn giữ lại các hành động
chính. Các nút trong bản xem trước không xin quyền mic hoặc phát/thu âm.

19 màn/trạng thái gồm danh sách máy iOS và Android, chi tiết máy, chờ kết nối,
lời mời chia sẻ, voice tắt/bật mic và tắt/bật loa độc lập, hai phiên ngang, hướng dẫn cử chỉ,
bàn phím bổ trợ, tài khoản, lỗi khởi động và xác nhận rời phiên. Đây là concept
client trên điện thoại; không mô tả iPhone điều khiển toàn bộ một iPhone khác.

## Đưa vào Figma

Cách ưu tiên: kết nối Figma trong cuộc trò chuyện rồi tạo file trực tiếp bằng
kết nối đó. Hiện chưa có URL file Figma vì tài khoản chưa kết nối.

Nếu muốn dựng offline qua Figma Desktop, thư mục `figma-plugin` có script tạo
Frame, Text, Rectangle, Ellipse và các liên kết prototype. Chữ và hình được tạo
thành lớp native; không phải ảnh chụp dán vào frame. Script không dùng mạng.

1. Trong Figma Desktop, tạo một development plugin mới để Figma cấp ID thật.
2. Chép `figma-plugin/code.js` vào thư mục plugin đó.
3. Dùng cấu hình từ `manifest.template.json`, bổ sung trường `id` do Figma vừa
   cấp. Không tự tạo ID hoặc dùng ID của plugin khác.
4. Chạy plugin trong một file thiết kế. Nó thêm một page mới, không xóa page cũ.

Tham khảo [manifest chính thức](https://developers.figma.com/docs/plugins/manifest/)
và [prototype reactions](https://developers.figma.com/docs/plugins/api/properties/nodes-reactions/).
Script đã kiểm tra cú pháp và dữ liệu liên kết, nhưng chưa chạy trong tài khoản
Figma thật. Chưa có Auto Layout, component variants hoặc design-system library.

## Phạm vi prototype

Các vùng tương tác ưu tiên: chọn Windows Studio → kết nối → phiên ngang →
bàn phím/cử chỉ/rời phiên; lời mời → xem chia sẻ → voice → bật/tắt mic và loa.
Tìm kiếm, các bộ lọc, card MacBook và các tùy chọn tài khoản hiện là phần trình bày,
chưa có logic dữ liệu. Đây không phải kiểm thử khả năng truy cập hay usability
trên điện thoại thật; cần duyệt lại kích thước chữ, bàn phím hệ thống, safe area
và thao tác một tay khi triển khai.

Toàn bộ tên người, nội dung máy tính, RTT, FPS và trạng thái mic là dữ liệu giả.
Không sửa hay đóng gói lại ứng dụng desktop hiện tại.
