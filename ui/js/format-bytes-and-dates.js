// Định dạng số liệu hiển thị. Dùng chung cho app chính và các hộp thoại.
"use strict";

const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** 1536 → "1.5 KB"; giữ 1 chữ số lẻ từ MB trở lên, 0 chữ số dưới đó. */
function fmtBytes(n) {
  if (!n) return "0 B";
  let i = 0;
  let v = n;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  const digits = i >= 2 ? (v < 10 ? 2 : 1) : 0;
  return `${v.toFixed(digits)} ${UNITS[i]}`;
}

/** Giây epoch → "3 ngày trước" / "2 months ago" theo ngôn ngữ. Trả "" nếu không rõ. */
function fmtAge(epochSec) {
  if (!epochSec) return "";
  const days = Math.floor((Date.now() / 1000 - epochSec) / 86400);
  if (days < 1) return t("age.today");
  if (days < 30) return t("age.days", { n: days });
  if (days < 365) return t("age.months", { n: Math.floor(days / 30) });
  return t("age.years", { n: Math.floor(days / 365) });
}

/** Artifact không sửa quá 30 ngày → gợi ý "lâu không đụng". */
function isStale(epochSec) {
  return epochSec > 0 && Date.now() / 1000 - epochSec > 30 * 86400;
}

/** Rút gọn đường dẫn Windows để hiển thị: bỏ tiền tố thư mục người dùng. */
function shortPath(p) {
  return String(p).replace(/^[A-Z]:\\Users\\[^\\]+\\/i, "~\\");
}

/** Chống chèn HTML khi nhét chuỗi từ backend vào innerHTML. */
function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}
