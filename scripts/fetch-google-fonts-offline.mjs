// Tải các subset latin/latin-ext/vietnamese của 3 font dùng trong app về ui/fonts/
// và sinh ui/css/fonts.css với @font-face trỏ tới file cục bộ, để app chạy không cần mạng.
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const fontsDir = path.join(here, "..", "ui", "fonts");
const cssOut = path.join(here, "..", "ui", "css", "fonts.css");
await mkdir(fontsDir, { recursive: true });

const UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36";
const families = [
  "Bricolage+Grotesque:opsz,wght@12..96,300..800",
  "Be+Vietnam+Pro:ital,wght@0,400;0,500;0,600;1,400",
  "JetBrains+Mono:wght@400;500;600",
];
const url = `https://fonts.googleapis.com/css2?${families.map((f) => `family=${f}`).join("&")}&display=swap`;
const css = await (await fetch(url, { headers: { "User-Agent": UA } })).text();

const wanted = new Set(["latin", "latin-ext", "vietnamese"]);
const blocks = css.split("/* ").slice(1);
let out = "";
let n = 0;
for (const block of blocks) {
  const subset = block.slice(0, block.indexOf(" */"));
  if (!wanted.has(subset)) continue;
  const face = block.slice(block.indexOf("@font-face"));
  const m = face.match(/url\((https:[^)]+)\)/);
  if (!m) continue;
  const family = face.match(/font-family: '([^']+)'/)[1].replace(/\s+/g, "-").toLowerCase();
  const style = face.match(/font-style: (\w+)/)[1];
  const weight = face.match(/font-weight: ([\d ]+)/)[1].replace(/\s+/g, "-");
  const file = `${family}-${style}-${weight}-${subset}.woff2`;
  const buf = Buffer.from(await (await fetch(m[1])).arrayBuffer());
  await writeFile(path.join(fontsDir, file), buf);
  out += face.replace(m[1], `../fonts/${file}`).trimEnd() + "\n";
  n++;
}
await writeFile(cssOut, `/* Sinh bởi scripts/fetch-google-fonts-offline.mjs — không sửa tay. */\n${out}`);
console.log(`đã ghi ${n} @font-face vào ${cssOut}`);
