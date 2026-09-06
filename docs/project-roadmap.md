# SLClean — hướng phát triển

Mục tiêu: từ "công cụ cho người biết máy" thành "ai cũng dùng được", mà không bỏ mất thứ
đang làm nên khác biệt. Ba phần: chỗ đứng hiện tại (đối chiếu mã), bản đồ thị trường (tra
cứu), rồi đề xuất chia bậc theo mức đáng làm.

Soát ngày 06/09/2026 trên v0.3.0: 7 tab, 26 lệnh backend, 74 test chạy mặc định.

## 1. Chỗ đứng hiện tại

### Đang hơn thị trường

- **Biết máy người làm kỹ thuật.** Catalog nhận cache của AI tool, package manager, build
  artifact, index của IDE (`catalog_specs.rs`, `artifacts.rs`). CCleaner, BleachBit và
  Storage Sense của Windows không biết những chỗ này — đúng chỗ tốn nhiều GB nhất.
- **File trùng so từng byte.** Ba tầng: kích thước → 4 KB đầu → SHA-256 toàn file
  (`duplicates.rs`). WizTree so tên + kích thước + ngày sửa, nên có thể gọi trùng thứ không
  trùng — mà xoá nhầm ở đây là mất dữ liệu thật.
- **Không tick hộ bao giờ.** Dữ liệu người dùng không tự tick; tick sạch cả một nhóm trùng
  thì phải qua cảnh báo (`duplicates-tab.js`).
- **Chặn ở backend, không chỉ ở giao diện.** Danh sách đường dẫn bảo vệ và
  `PROGRAM_FILES_KEEP` nằm trong Rust, nên một lỗi UI không dẫn tới xoá thư mục hệ thống.
- **Quét file lớn không cần admin, không đọc MFT** (`big_files.rs`). WizTree cần cả hai;
  đổi lại mình chậm hơn nên quét chạy nền, phát dần, huỷ được.
- **Song ngữ VI/EN đầy đủ**, kể cả câu lỗi. Gần như mọi công cụ cùng loại chỉ có EN.
- **MIT, không telemetry, không tài khoản, không bản Pro.** WizTree miễn phí cho cá nhân
  nhưng tính phí khi dùng thương mại.

### Thị trường có, mình chưa có

- Không có màn hình đầu trả lời "giờ tôi nên làm gì": mở app ra là sổ cái rỗng, phải tự
  bấm quét từng tab.
- Dọn xong không lưu lại gì. `clean()` trả `CleanResult` (freed/removed/skipped) rồi bỏ —
  không có sổ dọn, không có "lần trước lấy lại bao nhiêu", không có đường hoàn lại.
- Không xuất được báo cáo (CSV/HTML). Người làm dịch vụ cần đưa bằng chứng cho khách.
- Không có treemap. WinDirStat, WizTree, TreeSize đều có; bản đồ hiện tại là danh sách thanh.
- Không gỡ hàng loạt app. Revo và Bulk Crap Uninstaller xếp hàng gỡ liên tiếp được.
- Không dọn thành phần Windows (WinSxS, driver cũ, hibernation) — chỗ này 5–20 GB là thường.
- Không hẹn giờ, không tự dọn. Storage Sense có, dù chỉ cho Temp và Downloads.

### Chỗ thị trường còn hở

- Không công cụ nào nhìn theo **dự án**: "thư mục này 8 tháng không ai sửa, `target/` và
  `node_modules/` đang giữ 4.2 GB".
- Không công cụ nào **giải thích** trước khi xoá theo kiểu người thường hiểu; hoặc bảng
  liệt kê thô, hoặc một nút "Tối ưu" không nói nó làm gì.
- **Hoàn lại sau khi dọn** gần như không ai có.
- Nhật ký so hai lần quét thì **đã có nhiều nơi làm** (DiskBrain, SpaceRover, delta,
  DiskGeek, SizeTrend, và `dfp.exe` của chính Windows). Không coi đây là điểm độc quyền.

## 2. Bản đồ thị trường

| Công cụ | Mạnh | Hở |
|---|---|---|
| WizTree | Đọc MFT nên quét ổ NTFS trong vài giây, treemap | Cần admin + NTFS; trùng lặp chỉ so tên/cỡ/ngày; tính phí khi dùng thương mại |
| TreeSize / WinDirStat | Bản đồ dung lượng quen tay, lâu năm | Chỉ xem, không dọn theo hiểu biết; không biết cache của dev |
| CCleaner | Ai cũng biết tên, dọn rộng | Quảng cáo và upsell, telemetry, dọn registry làm điểm bán, không hoàn lại |
| BleachBit | Nguồn mở, nhiều luật dọn | Giao diện khó với người thường, không có sổ dọn, không xem theo ổ |
| PrivaZer | Sâu về dấu vết riêng tư, có sao lưu | Nặng, nhiều lựa chọn khó hiểu, dễ mạnh tay |
| FluentCleaner | WinUI 3, nguồn mở, luật do cộng đồng góp | Còn mới, phạm vi hẹp |
| Revo / Bulk Crap Uninstaller | Gỡ hàng loạt, quét vết còn sót sau gỡ | Chỉ về gỡ app; không đo ổ, không trùng lặp |
| Czkawka / dupeGuru | Trùng lặp mạnh, có hardlink, ảnh gần giống | Không dọn cache, không quản app, không nhắm Windows trước |
| Storage Sense | Có sẵn trong Windows, tự chạy | Chỉ Temp/Downloads/OneDrive; không biết cache dev; không giải thích |

Kết luận: phần "đo và xoá" thì thị trường đông. Phần **giải thích, ghi lại, hoàn lại và
nhìn theo dự án** thì gần như trống. Đó là hướng nên đi.

## 3. Bậc 1 — làm trước, đổi trải nghiệm nhiều nhất

### 1.1 Tab Tổng quan

Màn hình mở app: mỗi ổ một thanh đầy/trống, con số "có thể lấy lại khoảng X GB", Thùng rác,
và ba thẻ bấm một lần — *Dọn an toàn*, *Xem file lớn nhất*, *App lâu không mở*. Không bảng,
không nhãn kỹ thuật.

- **Vì sao hơn:** WinDirStat/WizTree ném treemap vào mặt người dùng; CCleaner ném con số
  "issues" gây lo. Không ai trả lời "giờ tôi nên làm gì".
- **Móc vào:** pane mới + `list_drives`, `recycle_bin_info`, tổng kết của các lần quét đã có.
- **Rủi ro:** con số "có thể lấy lại" phải đúng nghĩa *đã đo*, không đoán. Chưa quét thì ghi
  rõ là chưa quét.

### 1.2 Sổ dọn và hoàn lại

Mỗi lần dọn ghi một manifest (`runs/<mốc-thời-gian>.json`: mục nào, đường dẫn nào, bao nhiêu
byte, vào Thùng rác hay xoá thẳng). Từ đó có tab/mục "đã dọn": lần trước lấy lại bao nhiêu,
gồm những gì. Với các lần đi qua Thùng rác thì cho **hoàn lại**.

- **Vì sao hơn:** đây là rào cản niềm tin lớn nhất của người không hiểu máy. CCleaner không
  hoàn lại được; PrivaZer có sao lưu nhưng nặng và khó hiểu.
- **Móc vào:** `cleaner.rs` đã trả đủ số liệu từng mục, chỉ chưa lưu.
- **Giới hạn phải nói thật:** xoá thẳng thì không hoàn lại được. Sổ dọn phải ghi rõ lần đó
  dùng chế độ nào, và nên đề nghị bật Thùng rác cho người mới.

### 1.3 Chế độ Đơn giản / Chế độ Đầy đủ

Một công tắc trong Cài đặt. **Đơn giản**: chỉ hiện nhóm *tạo lại được*, ẩn nhãn an toàn và
cột kỹ thuật, mỗi mục một câu tiếng người ("cache của Chrome — xoá xong Chrome tự tạo lại,
lần mở web đầu hơi chậm"), một nút *Dọn an toàn*. **Đầy đủ**: đúng như bây giờ.

- **Vì sao hơn:** cách duy nhất mở rộng người dùng mà không làm app dốt đi. BleachBit mất
  người dùng thường vì bắt ai cũng đọc bảng kỹ thuật.
- **Móc vào:** một cờ trong `settings.rs`, một lớp CSS ở `<body>`, một nhánh trong
  `passesFilter`.
- **Rủi ro:** Chế độ Đơn giản vẫn **không được tick hộ** dữ liệu người dùng. Ẩn bớt thì
  được, tự quyết định thì không.

### 1.4 Lấy lại theo dự án

Gom artifact theo thư mục dự án, xếp theo lần sửa cuối: "D:\Project\cũ — 8 tháng không sửa —
`target/` 3.1 GB, `node_modules/` 1.1 GB". Một hàng, một tick, dọn cả dự án.

- **Vì sao hơn:** chưa thấy công cụ nào làm. Người viết code có 20–50 thư mục dự án cũ, mỗi
  cái vài GB, và không biết cái nào còn cần.
- **Móc vào:** `project_roots.rs` đã tìm ra thư mục dự án, `artifacts.rs` đã gán nhãn theo
  dự án. Ngày sửa cuối đọc từ mtime của `.git/refs` và `.git/HEAD`, không cần gọi `git`.
- **Rủi ro:** "lâu không sửa" không có nghĩa là bỏ. Chỉ xếp hạng và nói rõ, không tick hộ.

### 1.5 Ngân sách chỗ trống (tự nguyện bật)

"Giữ ổ C: luôn còn ít nhất 20 GB." Xuống dưới ngưỡng thì thông báo, và nếu người dùng đã
đồng ý trước thì tự dọn — **chỉ** nhóm *tạo lại được*, không bao giờ chạm nhóm *xem lại*.

- **Vì sao hơn:** Storage Sense có ý này nhưng chỉ biết Temp, Downloads và OneDrive, không
  biết cache dev — vốn là chỗ phình nhanh nhất.
- **Móc vào:** cờ trong `settings.rs` + một scheduled task gọi `slclean.exe --auto`; cần
  thêm chế độ chạy không mở cửa sổ.
- **Rủi ro:** cao nhất trong bậc này. Mặc định tắt, giới hạn cứng ở nhóm an toàn, luôn ghi
  vào sổ dọn (1.2), và báo sau khi chạy.

### 1.6 Xuất báo cáo và treemap

Xuất kết quả quét ra CSV/HTML (một file, mở được bằng trình duyệt), và thêm treemap cho bản
đồ ở tab File lớn.

- **Vì sao hơn:** treemap là thứ người dùng mong có sẵn — thiếu nó là điểm trừ so với mọi
  công cụ đo ổ. Bản xuất thì phục vụ thợ sửa máy và IT nội bộ: đưa được bằng chứng "đã lấy
  lại 42 GB, gồm những gì".
- **Móc vào:** dữ liệu bản đồ 3 tầng của `big_files.rs` đã đủ để vẽ; xuất báo cáo lấy từ
  `items` và sổ dọn.
- **Rủi ro:** thấp.

## 4. Bậc 2 — làm sau, đều là mở rộng của thứ đã có

- **Gỡ hàng loạt + quét vết sau gỡ.** Xếp hàng nhiều app, gỡ liên tiếp, gỡ xong tự chạy quét
  Thư mục thừa để bắt phần trình gỡ bỏ lại. Revo và BCU có; mình đã có cả `uninstall_app` và
  `leftovers.rs`, chỉ thiếu hàng đợi.
- **Danh sách giữ khi xoá cache trình duyệt.** Người dùng chọn miền cần giữ đăng nhập; dọn
  cache mà không đăng xuất. Chưa thấy công cụ nào làm tử tế.
- **Trùng lặp: hardlink thay vì xoá.** Cùng ổ NTFS thì giữ cả hai đường dẫn nhưng chỉ chiếm
  một bản. Czkawka có trên Linux, phía Windows GUI gần như không.
- **Dọn thành phần Windows bằng lệnh chính hãng.** Gói lại `DISM /StartComponentCleanup`,
  `pnputil` xoá driver cũ, `powercfg /hibernate off`, Windows Update cache — luôn nói rõ đang
  gọi lệnh gì của Microsoft, không tự tay xoá WinSxS.
- **Dấu vết riêng tư.** Recent files, jump list, thumbnail cache, clipboard, index Search:
  đều tạo lại được, đều đo được.
- **Thư mục rỗng và shortcut chết.** Ít dung lượng nhưng thấy được kết quả ngay; mình đã có
  bộ đọc `.lnk` (MS-SHLLINK) nên phát hiện shortcut trỏ vào chỗ không còn là gần như miễn phí.
- **Ngôn ngữ do cộng đồng góp.** Tách bảng chữ ra file JSON ngoài để ai cũng thêm được ngôn
  ngữ mà không cần build. FluentCleaner đi đường luật-dọn-cộng-đồng; mình đi đường ngôn ngữ.
- **Pháp y sau khi cài/cập nhật.** Chụp snapshot trước và sau khi cài một app rồi so, để
  biết app nào phình. Dựa trên `history.rs` đã có.

## 5. Bậc 3 — cân nhắc, rủi ro cao hoặc lệch phạm vi

- **Cập nhật app qua `winget`.** Hữu ích nhưng lệch khỏi việc "dọn", và lỗi cập nhật sẽ bị
  quy cho mình.
- **Chuyển thư mục sang ổ khác bằng junction.** Rất mạnh với ổ C: nhỏ, nhưng app tự cập nhật
  có thể phá liên kết. Nếu làm thì phải cảnh báo thẳng và có đường trả lại.
- **Ảnh gần giống nhau (perceptual hash).** Muốn thì phải tách khỏi tab File trùng, vì tab đó
  đang hứa "giống hệt từng byte" — trộn vào là phá lời hứa an toàn.

## 6. Không nên làm

- **Dọn registry làm điểm bán.** Lợi ích không đo được, rủi ro thật. Đây là chỗ khiến cả
  ngành mất uy tín.
- **Nút "Tối ưu" kiểu placebo**: dọn RAM, tăng tốc, "sửa 3.482 lỗi".
- **Telemetry, tài khoản, bản Pro khoá tính năng.** Đang là lợi thế, đừng bỏ.
- **Tick hộ dữ liệu người dùng**, ở bất kỳ chế độ nào.
- **Nhận là công cụ đầu tiên so được hai lần quét.** Nhiều nơi đã làm (xem §1). Chú thích
  đầu `history.rs` hiện đang nói quá chỗ này, nên sửa.

## 7. Thứ tự đề xuất

1.2 sổ dọn → 1.1 tổng quan → 1.3 chế độ đơn giản → 1.6 treemap và xuất báo cáo → 1.4 theo dự
án → 1.5 ngân sách chỗ trống. Sổ dọn đi trước vì tổng quan cần số liệu của nó, và vì mọi
tính năng tự động sau này đều phải ghi lại được mới dám bật.

## Nguồn

Đặc điểm từng công cụ tra từ trang chủ hoặc repo chính thức, 09/2026: wiztree.com,
windirstat.net, jam-software.com/treesize, ccleaner.com, bleachbit.org, privazer.com,
github.com/builtbybel/FluentCleaner, revouninstaller.com,
github.com/Klocman/Bulk-Crap-Uninstaller, github.com/qarmin/czkawka,
github.com/arsenetar/dupeguru, tài liệu Storage Sense của Microsoft. Số liệu về SLClean lấy
từ mã trong repo này, không từ trí nhớ.



