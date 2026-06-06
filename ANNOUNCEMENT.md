# 🎼 KJ CLIController: Biến *bất kỳ* AI agent nào thành nhạc trưởng điều phối cả đội CLI

Bạn có Claude Code, Codex, và Antigravity (Gemini) cài sẵn trên máy. Mỗi ông một thế mạnh — nhưng từ trước tới giờ chúng làm việc **riêng lẻ**, không "biết" đến nhau. Phiên bản mới của **KJ CLIController** thay đổi điều đó: giờ bạn có thể để **một** agent làm **nhạc trưởng**, lên kế hoạch rồi **giao việc cho những ông còn lại** — và tất cả cùng hiểu nhau qua từng bước.

## Vấn đề: nhiều bộ não, không trí nhớ chung

Gateway vốn là một API tương thích OpenAI, định tuyến tới các AI CLI local. Nhưng nó **stateless** — mỗi lần gọi là một tiến trình mới, không nhớ gì. Vậy làm sao để 3 agent khác hãng *cộng tác qua nhiều bước* mà vẫn nối tiếp được công việc của nhau?

## Lời giải: bộ nhớ chung nằm trên ổ đĩa

Thay vì nhồi nhét lịch sử hội thoại (tốn token, phình to), các agent chia sẻ ngữ cảnh qua **một file ledger `AGENT_LOG.md`** ngay trong thư mục dự án:

> Mỗi worker **đọc `AGENT_LOG.md` trước** để nắm việc các bước trước → làm phần của mình → **ghi append** lại tóm tắt + quyết định. Nhạc trưởng đọc lại file đó trước khi lên bước kế.

Đơn giản, nhưng mạnh: ngữ cảnh **bền vững** (sống sót cả khi restart), **portable** (chỉ là một file), **tiết kiệm quota** (đọc bản tóm tắt gọn thay vì cả lịch sử), và **chạy đều cho cả 3 CLI** mà không cần server phải nhớ gì.

## Khởi động chỉ bằng một đường link

Điểm hay nhất: bạn **không cần cài đặt gì trong dự án trước**. Mở agent bạn đang dùng, dán câu này:

> *"Đọc `http://localhost:8080/agents` rồi áp dụng vào repo này để điều khiển các AI CLI khác."*

- **`/agents`** trả về một protocol markdown thuần — agent đọc-hiểu là làm được ngay.
- **`/consult.py`** cho agent tải về công cụ điều phối bằng đúng một lệnh `curl`. Không cần dependency, chạy trên mọi máy/OS.

## Những điểm đáng giá

- 🤝 **Orchestrator-agnostic** — nhạc trưởng có thể là Antigravity, Claude, hay Codex. Không khoá cứng vào một hãng.
- 🌍 **Portable thật sự** — clone sang máy khác vẫn chạy: đường dẫn tương đối, tự dò CLI khả dụng, fallback nhẹ nhàng nếu thiếu.
- 🧠 **Tận dụng đúng thế mạnh**: **Claude** làm cố vấn kỹ thuật cấp cao, **Codex** bao quát ngữ cảnh & soát mạch lạc, **Gemini/Antigravity** đầy đủ năng lực cho việc bạn chờ được.
- 💸 **Tiết kiệm quota** — giao việc cho cặp Claude + Codex bằng `--models claude,openai`, worker chỉ đọc ledger gọn.
- 🛡️ **Bền với lỗi tạm thời** — auto-retry khi gặp phản hồi rỗng, 5xx, hay rớt mạng; không phí lần thử cho lỗi quota/token.
- 🔒 **Local-first** — mọi prompt chạy trên chính máy bạn, không gọi cloud trực tiếp.

## Đã được kiểm chứng thật

Trong một phiên relay 3 bước (Codex lập kế hoạch → Claude implement → Gemini finalize & verify), một agent đặt ra **quy ước tùy ý** (tên hàm + quy tắc đếm), và các agent sau **dùng đúng** quy ước đó — bằng chứng cứng rằng chúng **thực sự đọc ngữ cảnh của nhau**, chứ không đoán mò. Sản phẩm cuối chạy đúng ngay lần đầu.

## Bắt đầu

```bash
# 1. Chạy gateway
cargo run --release        # hoặc clicontroller.exe (chạy ẩn dưới system tray)

# 2. Mở agent bất kỳ, dán:
#    "Đọc http://localhost:8080/agents rồi áp dụng vào repo này."
```

⭐ Mã nguồn & hướng dẫn: **https://github.com/kentjuno/KJCLIController**

*Một API. Mọi AI CLI local. Giờ thì biết phối hợp với nhau.*
