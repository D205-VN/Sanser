# Server trên Mac, kết nối trực tiếp trước

Mac có thể chạy API đăng nhập, ghép phiên, signaling và relay dự phòng của Sanser.
Dữ liệu tài khoản tiếp tục nằm trên PostgreSQL/Neon cấu hình trong `.env`;
video và input không đi qua cơ sở dữ liệu.

Luồng ưu tiên:

```text
Mac client <--------- UDP trực tiếp ---------> Windows host
     \                                         /
      +---- HTTPS/WSS ---- server trên Mac ----+
                    đăng nhập / ghép phiên
```

- Cùng LAN: thử địa chỉ LAN của hai máy trước.
- Khác mạng: STUN tìm địa chỉ UDP ngoài NAT, hai máy gửi probe có xác thực để mở đường trực tiếp.
- Khi NAT/firewall không cho phép: dùng relay WebSocket, dữ liệu phiên được mã hóa đầu cuối.
- Đổi cổng không tự mở NAT. Khi không có quyền cấu hình router, tunnel HTTPS chỉ giải quyết truy cập server; nó không biến relay thành UDP trực tiếp.

## Chạy server và tunnel thử nghiệm

Yêu cầu: macOS, Node >= 20.19, Cargo, `cloudflared`, `.env` hợp lệ với
`DATABASE_URL` và `SESSION_CREDENTIAL_KEY`. Không thay `.env` bằng file mẫu nếu đã có tài khoản.

Lệnh sau biên dịch server release, cài LaunchAgent của người dùng và mở
Cloudflare Quick Tunnel công khai. Chỉ chạy khi đã chọn dùng Cloudflare và tự khởi động:

```sh
npm run server:selfhost -- install
npm run server:selfhost -- status
```

Server bind `127.0.0.1:5174`; Cloudflare cấp URL HTTPS tạm thời.
`localReady` và `publicReady` cần đều là `true` trước khi chuyển hai máy.
Log và địa chỉ nằm trong `data/self-host/` (không đưa lên Git).
Dịch vụ chạy khi đăng nhập macOS, giữ Mac không tự idle sleep khi còn mở nắp;
đóng nắp, tắt máy hoặc mất Internet sẽ làm server không truy cập được.

Bản ứng dụng có thay đổi này cho phép mở **Settings → Network → Server settings**,
hoặc **Server settings** ở màn hình đăng nhập. Nhập cùng URL HTTPS trên **cả hai máy**,
đăng nhập lại và chọn **Auto**. Bản 2.1.7 phát hành trước đó chưa có lựa chọn
server riêng trong bản đóng gói. Không thay mặc định Render toàn bộ người dùng
bằng một URL thử nghiệm.

Quick Tunnel không có địa chỉ cố định hoặc cam kết uptime. Khi dịch vụ/tunnel
khởi động lại, chạy `status` và cập nhật URL trên hai máy. Để dùng lâu dài,
cần named tunnel với tên miền cố định hoặc một mạng riêng được cấu hình cho cả hai máy.

Dừng và bỏ tự khởi động:

```sh
npm run server:selfhost -- stop
```

## Đo trước khi kết luận độ trễ

Mục tiêu 5–15 ms cần tách rõ RTT mạng và tổng input-to-display.
60 FPS có khoảng khung hình 16,7 ms; capture, encode, truyền, decode và hiển thị
đều góp phần vào độ trễ cảm nhận. Đặt server gần không loại bỏ những phần này.

1. Kiểm tra phiên hiện là **Direct** hay **Relay** trong Diagnostics.
2. Thử cùng LAN, ưu tiên Ethernet hoặc Wi-Fi ổn định; đo lại cả màn hình tĩnh và chuyển động.
3. Xuất Diagnostics để so sánh RTT, thời gian encode/decode, hàng đợi và dropped frames.
4. Sau đó mới thử hai mạng khác nhau. Không suy ra kết quả WAN từ loopback hoặc LAN.

Bản sửa probe giữ việc thăm dò UDP đến hết ngân sách kết nối thay vì bỏ cuộc sau
1,8 giây. Kiểm thử tái hiện máy bên kia bắt đầu chậm 2,2 giây đã thất bại trước
bản sửa và thành công sau bản sửa. Đây là kiểm chứng tránh fallback sai;
chưa phải phép đo độ trễ Mac–Windows thực tế.

Tài liệu Cloudflare: https://developers.cloudflare.com/tunnel/get-started/quick-tunnels/
