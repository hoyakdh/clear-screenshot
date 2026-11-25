// Which app a screenshot was taken in (Instagram, YouTube, a web browser, KakaoTalk …).
// Recognized from tell-tale interface text found by text recognition; the on-device model's guess
// is used only when it names one of these apps exactly (it tends to invent generic answers).
//
// Each rule: `cues` (regex sources, case-insensitive). A screenshot matches when at least `min`
// different cues are present. Earlier rules win, so specific apps come before "web browser".

const APP_RULES = [
  { key: "instagram", name: "Instagram", aliases: ["인스타그램", "인스타", "insta"], min: 1,
    cues: ["\\binstagram\\b", "^릴스$", "좋아요\\s*[\\d,.]+\\s*개", "댓글\\s*[\\d,.]+\\s*개\\s*모두\\s*보기", "\\breels\\b", "회원님을 위한 추천"] },
  { key: "youtube", name: "YouTube", aliases: ["유튜브", "유투브"], min: 1,
    cues: ["\\byoutube\\b", "조회수\\s*[\\d,.]+\\s*(만|천|억)?\\s*회", "구독자\\s*[\\d,.]+", "\\bshorts\\b"] },
  { key: "tiktok", name: "TikTok", aliases: ["틱톡"], min: 1, cues: ["\\btiktok\\b", "^팔로잉\\s*\\|?\\s*추천$"] },
  { key: "x", name: "X (Twitter)", aliases: ["트위터", "twitter", "엑스"], min: 1, cues: ["재게시", "\\bretweet", "트윗"] },
  { key: "threads", name: "Threads", aliases: ["스레드"], min: 1, cues: ["\\bthreads\\b", "스레드에서"] },
  { key: "kakaotalk", name: "KakaoTalk", aliases: ["카카오톡", "카톡"], min: 1, cues: ["\\bkakaotalk\\b", "카카오톡", "메시지 입력", "오픈채팅"] },
  { key: "messenger", name: "Messenger", aliases: ["메신저", "채팅", "chat"], min: 99, cues: [] },
  { key: "messages", name: "Messages", aliases: ["메시지", "문자", "imessage"], min: 1, cues: ["\\bimessage\\b", "문자 메시지", "\\[web발신\\]"] },
  { key: "coupang", name: "Coupang", aliases: ["쿠팡"], min: 1, cues: ["쿠팡", "\\bcoupang\\b", "로켓배송"] },
  { key: "toss", name: "Toss", aliases: ["토스"], min: 1, cues: ["^토스$", "\\btoss\\b", "토스뱅크", "토스페이"] },
  { key: "naver", name: "NAVER", aliases: ["네이버"], min: 1, cues: ["\\bnaver\\b", "네이버"] },
  { key: "maps", name: "Maps", aliases: ["지도", "map", "길찾기"], min: 2, cues: ["길찾기", "출발", "도착", "\\bkakaomap\\b", "네이버 지도", "도보\\s*\\d+\\s*분", "\\b\\d+\\s*km\\b"] },
  { key: "code", name: "Code editor", aliases: ["코드", "에디터", "vscode", "cursor", "xcode", "terminal", "터미널"], min: 1,
    cues: ["visual studio code", "\\bxcode\\b", "\\bzsh\\b", "\\bterminal\\b", "^\\s*(fn|def|const|let|import|function|class)\\s+\\w+", "cargo (build|run|test)", "\\bnpm (run|install)\\b"] },
  { key: "browser", name: "Web browser", aliases: ["웹", "브라우저", "웹브라우저", "safari", "chrome", "사파리", "크롬", "web"], min: 1,
    cues: ["https?://", "^\\s*(www\\.|m\\.)?[a-z0-9-]+(\\.[a-z0-9-]+)*\\.(com|net|org|io|kr|co\\.kr|dev|app|me|ai)(/\\S*)?\\s*$", "🔒"] },
];

const APP_BY_KEY = Object.fromEntries(APP_RULES.map((r) => [r.key, r]));
const APP_RE = APP_RULES.map((r) => ({ ...r, re: r.cues.map((c) => new RegExp(c, "im")) }));
const appNorm = (s) => (s || "").normalize("NFC").toLowerCase().replace(/[\s()]+/g, "");

/** When no interface text gives the app away, the kind of screen the model saw still narrows it. */
const KIND_TO_APP = { web_page: "browser", chat: "messenger", code: "code" };

/** App key for a screenshot from its recognized text, then the model's guess, then the screen kind. */
function detectApp(text, aiApp, aiKind) {
  const t = text || "";
  for (const r of APP_RE) {
    if (r.re.filter((re) => re.test(t)).length >= r.min) return r.key;
  }
  const guess = appNorm(aiApp);
  if (guess) {
    const hit = APP_RULES.find((r) => appNorm(r.name) === guess || r.aliases.some((a) => appNorm(a) === guess));
    if (hit) return hit.key;
  }
  return KIND_TO_APP[aiKind] || null;
}

/** Generic apps ("Web browser" …) are translated; product names stay as they are. */
const appName = (key) => {
  const k = `app.${key}`;
  const tr = typeof t === "function" ? t(k) : k;
  return tr !== k ? tr : APP_BY_KEY[key]?.name || key;
};

/** App keys a search query refers to ("인스타", "youtube", "브라우저" …). */
function appsForQuery(q) {
  const n = appNorm(q);
  if (n.length < 2) return [];
  return APP_RULES.filter((r) => [r.name, r.key, ...r.aliases].some((a) => appNorm(a).startsWith(n))).map((r) => r.key);
}
