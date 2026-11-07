# Nghiên cứu REA: Parsec và các engine tương tự Sanser

Ngày: 18/10/2026. Đối chiếu Sanser 2.2.15, commit `2e5767fab504dda35611d425a20fa7fda8865fd0`.

Đây là kết quả phân tích tĩnh đầu tiên, **không phải benchmark** hay khẳng định đã giải mã toàn bộ Parsec. Kết luận được tách thành dữ kiện kiểm tra được, suy luận và phép đo còn thiếu.

## Đã dùng REA như thế nào

REA CLI `6.5.0` được cài riêng trong thư mục tạm, không đăng ký MCP, không sửa cấu hình Codex/dependencies Sanser. Đã chạy thành công `providers`, `inspect-artifact` cho `/Applications/Parsec.app`, và `inspect-macho` cho launcher cùng thư viện core đi kèm. Những lệnh này gọi tiện ích phân tích của macOS; **không chạy ứng dụng Parsec**. Một lần gọi Mach-O đầu tiên bị chặn cache compiler; đã khắc phục bằng cache tạm rồi chạy thành công.

Ghidra, Hopper và IDA chưa được cấu hình. Vì vậy chưa có decompile, call graph đầy đủ hay trace thuật toán BUD. REA có thể tiến xa hơn khi bổ sung provider, nhưng kết quả hiện tại không được gọi là đã reverse toàn bộ engine.

Tham khảo [REA](https://github.com/morluto/rea) và [CLI](https://github.com/morluto/rea/blob/main/docs/cli.md). Bằng chứng đã rút gọn, hash và các commit nguồn nằm trong [evidence JSON](rea-parsec-evidence-2026-10-18.json). Kết quả REA đầy đủ giữ tại `/private/tmp/sanser-rea-research-2i98__9a` trên máy này; không đưa binary thương mại hoặc toàn bộ mã nguồn đối thủ vào dự án.

## 1. Parsec: những gì đã kiểm tra trực tiếp

Bản bundle: **150.101.1**, ID `tv.parsec.www`.

| Đối tượng | SHA-256 | Kết quả |
| --- | --- | --- |
| `Contents/MacOS/parsecd` | `785f985d7f0cf6cffda8966b776a61b6c6cb0c79e582a8e3f35cda36305be76d` | Launcher Mach-O universal x86_64/arm64 |
| `Contents/Resources/skel/parsecd-150-101a.dylib` | `9cce85059c7f95f1a8e578b3c9f7200cb364520e557b5c85294802c8c596cdd5` | Core Mach-O; đã phân tích slice arm64 |

Phân tích riêng launcher sẽ bỏ sót phần media. Core liên kết ScreenCaptureKit, VideoToolbox, Metal/IOSurface và AudioToolbox/CoreAudio. Các import gồm `VTCompressionSessionEncodeFrame`, `VTDecompressionSessionDecodeFrame`, `AudioQueueNewOutput`, `AudioQueueEnqueueBuffer`, `sendto` và `recvfrom`.

Core còn có các chuỗi `opus_encode`, `opus_decode`, `network_cg_level`, `network_fast_relay_ping`, `network_upnp`, `app_stun_address` và `libminiupnpc`. Offset byte được ghi trong evidence. **Sự hiện diện của chuỗi/import không chứng minh đường gọi đang chạy, cấu hình đang bật, zero-copy hay độ trễ thực tế.** Thư viện skel đi kèm bundle cũng có thể khác core được cập nhật khi chạy; không đọc thư mục dữ liệu tài khoản để tìm bản đang dùng.

Tài liệu Parsec xác nhận API/backend qua TCP/443, STUN qua UDP và media P2P qua UDP; enterprise relay nhận/chuyển UDP. HPR đo các relay candidate để chọn đường có độ trễ thấp. Đây là tài liệu sản phẩm, chưa phải quan sát phiên Parsec trên mạng của bạn. Nguồn: [Connectivity](https://support.parsec.app/hc/en-us/articles/32381460716180-Parsec-Connectivity-Requirements), [HPR selection](https://support.parsec.app/hc/en-us/articles/32381482907156-Deployment-Considerations-and-Options).

**Chưa biết:** công thức congestion controller, pacing theo frame/packet, cấu trúc ACK/retry BUD, deadline audio, cách phục hồi route giữa phiên, số lần encode khi nhiều viewer. Không suy ra các chi tiết này từ tên setting hoặc tuyên bố quảng bá.

## 2. Âm thanh: khoảng cách rõ nhất với Sanser

| Hệ thống | Bằng chứng đọc được | Bài học áp dụng |
| --- | --- | --- |
| Parsec | Core có dấu vết Opus và AudioQueue | Có cơ sở nghiên cứu codec/playout sâu hơn; chưa biết cấu hình thực thi |
| Sunshine | Encoder Opus multistream trên worker riêng, chế độ restricted-lowdelay; cấu hình stereo có 96 và 512 kbit/s | Tách thu, mã hóa và phát; thương lượng bitrate/channels |
| Moonlight | Queue RTP audio có Reed–Solomon FEC; packet thiếu được chuyển thành lời gọi decode với dữ liệu rỗng để kích hoạt Opus PLC | Reorder/repair có deadline và xử lý mất âm thanh, thay vì chỉ chèn im lặng |
| RustDesk | Audio service dùng Opus LowDelay, chuyển kênh và resampler có kích thước frame cố định | Kiểm tra 44,1/48/96 kHz và ranh giới callback, không giả định thiết bị luôn cùng format |
| Sanser 2.2.15 | SNV2 `SPA1`: PCM16, 48 kHz stereo, khối 5 ms; sửa lịch phát Windows và giới hạn hàng đợi Mac | Bản vá playout là cần thiết, nhưng chưa có Opus PLC/FEC tương đương |

Nguồn mã đã ghim commit: [Sunshine audio](https://github.com/LizardByte/Sunshine/blob/3d6e1d6008bd4a24906283ab25fcd0becc0a1c07/src/audio.cpp), [Moonlight audio](https://github.com/moonlight-stream/moonlight-common-c/blob/aeba988e0a302145eff2e70fa3e11cd98ec01f5d/src/AudioStream.c), [Moonlight RTP audio](https://github.com/moonlight-stream/moonlight-common-c/blob/aeba988e0a302145eff2e70fa3e11cd98ec01f5d/src/RtpAudioQueue.c), [RustDesk audio](https://github.com/rustdesk/rustdesk/blob/53996e3c404ea5083df0775057e4149e49102084/src/server/audio_service.rs), [RustDesk resampler](https://github.com/rustdesk/rustdesk/blob/53996e3c404ea5083df0775057e4149e49102084/src/audio_resampler.rs). [API Opus](https://opus-codec.org/docs/opus_api-1.6/group__opus__decoder.html) mô tả PLC khi đưa packet mất vào decoder bằng dữ liệu rỗng.

Tính riêng payload, PCM16 48 kHz stereo cần **1,536 Mbit/s**, chưa có header/mã hóa. Stereo Opus 96 kbit/s trong cấu hình Sunshine nhỏ hơn 16 lần; đó không phải dự báo chất lượng hoặc RTT của Sanser. Khi mạng đã nghẽn, băng thông audio cao và thiếu PLC có thể làm nghe ngắt/rè; đây là **giả thuyết cần đo**, không phải nguyên nhân đã chứng minh cho mọi lần rè.

Trong SNV2, `lastAudio_` hiện loại audio đến trễ hơn audio mới nhất; sequence của stream còn dùng chung với control. Vì vậy không thể đếm chênh lệch sequence đó thành số packet audio mất một cách trực tiếp. Một audio sequence/timestamp riêng là nền tảng tốt hơn cho reorder và PLC.

Đề nghị: thương lượng capability Opus mới, giữ PCM tương thích; sequence/timestamp audio riêng; reorder deadline ngắn; Opus decode tuần tự, PLC đúng thời lượng; khảo sát FEC sau. FEC RTP của Moonlight và in-band FEC của Opus là hai cơ chế khác nhau, không được đồng nhất. Mỗi viewer có decoder/playout/mute riêng.

## 3. Input: không nên hiểu “UDP = bỏ reliability”

Moonlight hiện gộp **delta tương đối** đang chờ và đưa mouse vào channel riêng. Ở commit đã đọc, mouse relative vẫn dùng ENet reliable; code còn ghi việc chuyển sang unreliable cần thêm xử lý để biết delta bị mất. Keyboard cũng reliable. Nguồn: [InputStream](https://github.com/moonlight-stream/moonlight-common-c/blob/aeba988e0a302145eff2e70fa3e11cd98ec01f5d/src/InputStream.c).

Điều này sửa một giả định quá đơn giản trong roadmap trước: “Moonlight luôn latest-state-wins cho chuột” là không đúng cho mọi loại input. Vị trí tuyệt đối/state có thể thay bản cũ; delta tương đối phải cộng dồn hoặc có thiết kế phát hiện mất. Key/button vẫn cần thứ tự, ACK và fail-safe release.

Sanser đã có lane Control/Input/Audio/Video/Repair và slot latest mouse; cần đo đến socket, không xây scheduler thứ hai chỉ vì đối thủ có channel. Kiểm tra xem head-of-line trong lane reliable, OS queue và callback có phải bottleneck thật trước khi đổi socket.

## 4. Video và chia sẻ nhóm

Moonlight kết hợp FEC với khôi phục reference/IDR; khi queue decode quá tải vẫn có đường bỏ queue và xin IDR. Nó không coi mọi P-frame là có thể bỏ tùy ý. Nguồn: [VideoDepacketizer](https://github.com/moonlight-stream/moonlight-common-c/blob/aeba988e0a302145eff2e70fa3e11cd98ec01f5d/src/VideoDepacketizer.c). Sanser 2.2.14–15 đã giữ chuỗi encoded reference ngắn thay cho slot duy nhất; vẫn cần test mất gói và viewer chậm trên thiết bị thật.

Sunshine có trạng thái video/audio theo session và socket sender riêng trong [stream.cpp](https://github.com/LizardByte/Sunshine/blob/3d6e1d6008bd4a24906283ab25fcd0becc0a1c07/src/stream.cpp), cùng encode session trong [video.cpp](https://github.com/LizardByte/Sunshine/blob/3d6e1d6008bd4a24906283ab25fcd0becc0a1c07/src/video.cpp). Không nên đồng nhất thiết kế đó với “mã hóa đúng một lần cho mọi viewer”. Kiến trúc một encoder của Sanser tiết kiệm GPU nhưng đánh đổi cấu hình/chất lượng và phục hồi reference giữa các viewer. Chưa có bằng chứng để tăng giới hạn 3 lên 10 an toàn.

## 5. Direct/Relay và lỗi RTT khoảng 200 ms

Sanser đã có cửa sổ đo nhiều candidate và score RTT/jitter/loss; không còn có thể mô tả đơn giản là candidate đầu tiên thắng. Phục hồi hiện là reconnect có giới hạn, chưa phải chuyển socket media liền mạch. Xem `crates/sanser-p2p/src/punch.rs` và [route recovery](../route-recovery.md).

Bài học từ Parsec là triển khai đường UDP thực, đo relay/location và tách control-plane khỏi media. Đổi vị trí auth/signaling sang Mac không tự giảm RTT của một phiên media Direct. Nếu cả ứng dụng RTT và same-socket echo đều khoảng 200 ms, phải đo endpoint/interface thực tế, VPN overlay, tải mạng và đường truyền trước khi thay buffer.

RustDesk có luồng rendezvous/punch/fallback relay tường minh trong [client](https://github.com/rustdesk/rustdesk/blob/53996e3c404ea5083df0775057e4149e49102084/src/client.rs); không nên mặc định toàn bộ transport của nó giống Parsec. Chưa trace runtime hai sản phẩm trên mạng của bạn nên chưa có bảng thắng/thua về latency.

## Thứ tự triển khai đề xuất sau nghiên cứu

| Bước | Công việc cụ thể | Điều kiện nghiệm thu |
| --- | --- | --- |
| 1 | Benchmark A/B Sanser và Parsec cùng hai máy, cùng network, cùng cấu hình | Ghi actual endpoint/route, RTT P50/P95, FPS/drop và độ trễ hình; không trộn ICMP tới modem/VPN với media RTT |
| 2 | Audio sequence/timestamp, thống kê underrun/reorder/loss và Opus + PLC có capability fallback | Voice/music 10 phút, jitter/loss/burst/stall mô phỏng; test 44,1/48/96 kHz, đổi loa USB/Bluetooth và reset generation |
| 3 | Nhận diện bottleneck Share bằng tiến triển capture → send → receive → decode → render | 1/2/3 viewer, một viewer bị hạn chế, UAC/pause và reconnect; viewer khỏe không bị keyframe storm |
| 4 | Public UDP relay + lựa chọn theo RTT; giữ WSS fallback | Test máy chủ có UDP ingress thật; tách rõ giới hạn triển khai hiện chưa có VPS/cổng public |
| 5 | Chuẩn hóa transport/diagnostics Legacy và SNV2 theo từng capability | Cả bốn chiều qua cùng ma trận, có rollback; không thay toàn bộ pipeline đồng thời |

Các bước này là đề xuất, **chưa được triển khai trong lượt nghiên cứu**. Không thay mã engine, phát hành bản mới hay thay bản cài trên máy bạn. Bước sâu tiếp theo cho Parsec là bổ sung decompiler rồi trace cụ thể Opus/playout/pacing; tiếp theo nữa mới đo runtime có phiên hai máy và ghi endpoint/timing.
