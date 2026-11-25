// "What is in the picture" search.
// The backend stores Vision image-classification ids per screenshot (e.g. "coffee", "cup").
// Categories below map everyday search words (ko/en/ja/es) onto those ids, so searching
// "커피" or "음료" finds screenshots whose picture shows coffee or drinks — and also ones whose
// recognized text says "아메리카노", "latte", … (ocr).

const OBJECT_CATEGORIES = [
  { key: "coffee", ids: ["coffee", "coffee_bean"],
    ocr: ["아메리카노", "라떼", "카페라떼", "에스프레소", "콜드브루", "카푸치노", "마끼아또", "americano", "latte", "espresso", "cappuccino", "cold brew", "macchiato"],
    label: { ko: "커피", en: "Coffee", ja: "コーヒー", es: "Café" },
    words: { ko: ["커피", "카페", "라떼", "아메리카노", "에스프레소", "카푸치노"], en: ["coffee", "cafe", "latte", "americano", "espresso", "cappuccino"], ja: ["コーヒー", "カフェ", "珈琲", "ラテ"], es: ["café", "cafe", "latte", "cortado"] } },
  { key: "tea", ids: ["tea_drink", "teapot", "bubble_tea"],
    ocr: ["녹차", "홍차", "밀크티", "버블티", "보바", "matcha", "말차", "milk tea", "bubble tea"],
    label: { ko: "티·차", en: "Tea", ja: "お茶", es: "Té" },
    words: { ko: ["녹차", "홍차", "밀크티", "버블티"], en: ["tea", "bubble tea", "boba"], ja: ["お茶", "紅茶", "タピオカ"], es: ["té", "te"] } },
  { key: "alcohol", ids: ["beer", "wine", "red_wine", "white_wine", "sparkling_wine", "wine_bottle", "cocktail"],
    ocr: ["맥주", "와인", "칵테일", "소주", "하이볼", "beer", "wine", "cocktail", "highball"],
    label: { ko: "술", en: "Alcohol", ja: "お酒", es: "Alcohol" },
    words: { ko: ["술", "맥주", "와인", "칵테일"], en: ["alcohol", "beer", "wine", "cocktail"], ja: ["お酒", "ビール", "ワイン", "カクテル"], es: ["alcohol", "cerveza", "vino", "cóctel"] } },
  { key: "drink", ids: ["drink", "coffee", "tea_drink", "bubble_tea", "juice", "smoothie", "soda", "milkshake", "cocktail", "beer", "wine", "red_wine", "white_wine", "sparkling_wine", "cup", "drinking_glass", "straw_drinking"],
    ocr: ["음료", "주스", "에이드", "스무디", "아메리카노", "라떼", "juice", "smoothie", "lemonade", "latte", "americano"],
    label: { ko: "음료", en: "Drinks", ja: "飲み物", es: "Bebidas" },
    words: { ko: ["음료", "음료수", "마실것", "드링크", "주스", "스무디"], en: ["drink", "drinks", "beverage", "juice", "smoothie", "soda"], ja: ["飲み物", "ドリンク", "ジュース"], es: ["bebida", "bebidas", "jugo", "zumo", "refresco"] } },
  { key: "dessert", ids: ["dessert", "cake", "cake_regular", "birthday_cake", "wedding_cake", "cheesecake", "cupcake", "cookie", "donut", "ice_cream", "frozen_dessert", "pastry", "chocolate", "candy", "pancake"],
    ocr: ["케이크", "디저트", "쿠키", "도넛", "아이스크림", "마카롱", "dessert", "cake"],
    label: { ko: "디저트", en: "Dessert", ja: "デザート", es: "Postres" },
    words: { ko: ["디저트", "케이크", "쿠키", "도넛", "아이스크림", "빵"], en: ["dessert", "cake", "cookie", "donut", "ice cream", "sweets"], ja: ["デザート", "ケーキ", "スイーツ", "アイス"], es: ["postre", "postres", "pastel", "galleta", "helado"] } },
  { key: "food", ids: ["food", "bread", "white_bread", "pizza", "hamburger", "sandwich", "sushi", "pasta", "salad", "soup", "rice", "meat", "steak", "fried_chicken", "grilled_chicken", "seafood", "dumpling", "egg", "fried_egg", "fruit", "vegetable", "meatball"],
    ocr: ["맛집", "배달음식", "레시피", "요리", "육수", "반찬", "찌개", "라면", "밀키트", "식단", "밥", "restaurant"],
    label: { ko: "음식", en: "Food", ja: "食べ物", es: "Comida" },
    words: { ko: ["음식", "요리", "밥", "맛집", "피자", "햄버거", "초밥", "파스타", "고기", "치킨", "과일", "샐러드"], en: ["food", "meal", "pizza", "burger", "sushi", "pasta", "meat", "chicken", "fruit", "salad"], ja: ["食べ物", "料理", "ご飯", "ピザ", "寿司", "肉"], es: ["comida", "pizza", "hamburguesa", "sushi", "carne", "fruta"] } },
  { key: "dog", ids: ["dog", "puppy"], ocr: [],
    label: { ko: "강아지", en: "Dogs", ja: "犬", es: "Perros" },
    words: { ko: ["강아지", "개", "멍멍이"], en: ["dog", "dogs", "puppy"], ja: ["犬", "いぬ", "イヌ"], es: ["perro", "perros", "cachorro"] } },
  { key: "cat", ids: ["cat", "kitten", "adult_cat"], ocr: [],
    label: { ko: "고양이", en: "Cats", ja: "猫", es: "Gatos" },
    words: { ko: ["고양이", "냥이"], en: ["cat", "cats", "kitten"], ja: ["猫", "ねこ", "ネコ"], es: ["gato", "gatos"] } },
  { key: "animal", ids: ["animal", "dog", "cat", "kitten", "bird", "fish", "horse", "rabbit"], ocr: [],
    label: { ko: "동물", en: "Animals", ja: "動物", es: "Animales" },
    words: { ko: ["동물", "새", "물고기"], en: ["animal", "animals", "pet", "bird", "fish"], ja: ["動物", "鳥", "魚"], es: ["animal", "animales", "mascota", "pájaro", "pez"] } },
  { key: "people", ids: ["people", "adult", "child", "baby", "crowd"], ocr: [],
    label: { ko: "사람", en: "People", ja: "人物", es: "Personas" },
    words: { ko: ["사람", "인물", "아이", "아기", "셀카"], en: ["people", "person", "child", "baby", "selfie"], ja: ["人", "人物", "子供", "赤ちゃん"], es: ["personas", "gente", "niño", "bebé"] } },
  { key: "vehicle", ids: ["car", "vehicle", "conveyance", "sportscar", "truck", "bus", "train", "train_real", "airplane", "aircraft", "bicycle", "motorcycle", "boat"], ocr: [],
    label: { ko: "탈것", en: "Vehicles", ja: "乗り物", es: "Vehículos" },
    words: { ko: ["차", "자동차", "탈것", "비행기", "기차", "자전거", "버스", "오토바이", "배"], en: ["car", "vehicle", "plane", "airplane", "train", "bike", "bus"], ja: ["車", "乗り物", "飛行機", "電車", "自転車"], es: ["coche", "auto", "vehículo", "avión", "tren", "bicicleta"] } },
  { key: "nature", ids: ["outdoor", "sky", "blue_sky", "night_sky", "sunset_sunrise", "mountain", "beach", "ocean", "lake", "river", "waterfall", "forest", "desert", "snow", "park"], ocr: [],
    label: { ko: "풍경", en: "Scenery", ja: "風景", es: "Paisajes" },
    words: { ko: ["풍경", "자연", "하늘", "바다", "산", "해변", "노을", "야경", "숲", "눈"], en: ["scenery", "nature", "sky", "sea", "beach", "mountain", "sunset", "forest", "snow"], ja: ["風景", "自然", "空", "海", "山", "夕焼け"], es: ["paisaje", "naturaleza", "cielo", "mar", "playa", "montaña", "atardecer"] } },
  { key: "plant", ids: ["plant", "flower", "flower_arrangement", "tree", "foliage", "garden", "decorative_plant", "sunflower"], ocr: [],
    label: { ko: "꽃·식물", en: "Plants", ja: "花・植物", es: "Plantas" },
    words: { ko: ["꽃", "식물", "나무", "정원"], en: ["flower", "flowers", "plant", "tree", "garden"], ja: ["花", "植物", "木"], es: ["flor", "flores", "planta", "árbol"] } },
  { key: "place", ids: ["building", "cityscape", "skyscraper", "house_single", "storefront", "restaurant", "interior_room", "interior_shop", "living_room", "bedroom", "kitchen", "dining_room", "furniture"], ocr: [],
    label: { ko: "장소·건물", en: "Places", ja: "場所・建物", es: "Lugares" },
    words: { ko: ["건물", "도시", "집", "인테리어", "가게", "식당", "방"], en: ["building", "city", "house", "interior", "room", "shop", "restaurant"], ja: ["建物", "街", "家", "部屋", "店"], es: ["edificio", "ciudad", "casa", "interior", "tienda", "restaurante"] } },
  { key: "fashion", ids: ["clothing", "footwear", "shoes", "sneaker", "bag", "jewelry", "hat", "jeans", "watch", "wedding_dress"],
    ocr: [],
    label: { ko: "옷·패션", en: "Fashion", ja: "ファッション", es: "Moda" },
    words: { ko: ["옷", "패션", "신발", "가방", "모자", "운동화", "쥬얼리"], en: ["clothes", "clothing", "fashion", "shoes", "bag", "hat", "sneakers"], ja: ["服", "ファッション", "靴", "バッグ"], es: ["ropa", "moda", "zapatos", "bolso"] } },
  { key: "electronics", ids: ["consumer_electronics", "computer", "laptop", "computer_monitor", "computer_keyboard", "phone", "camera", "headphones", "gamepad", "videogame"], ocr: [],
    label: { ko: "전자기기", en: "Electronics", ja: "電子機器", es: "Electrónica" },
    words: { ko: ["전자기기", "컴퓨터", "노트북", "폰", "카메라", "헤드폰", "게임"], en: ["electronics", "computer", "laptop", "phone", "camera", "headphones", "game"], ja: ["電子機器", "パソコン", "スマホ", "カメラ"], es: ["electrónica", "ordenador", "portátil", "teléfono", "cámara"] } },
  { key: "sport", ids: ["sport", "ball", "soccer", "basketball", "baseball", "tennis", "golf", "swimming", "skiing", "yoga", "sports_equipment", "football"], ocr: [],
    label: { ko: "운동", en: "Sports", ja: "スポーツ", es: "Deportes" },
    words: { ko: ["운동", "스포츠", "축구", "야구", "농구", "골프", "요가", "수영"], en: ["sport", "sports", "soccer", "football", "baseball", "golf", "yoga"], ja: ["スポーツ", "運動", "サッカー", "野球"], es: ["deporte", "deportes", "fútbol", "béisbol", "golf"] } },
  { key: "art", ids: ["art", "illustrations", "painting"], ocr: [],
    label: { ko: "그림·일러스트", en: "Art", ja: "イラスト", es: "Arte" },
    words: { ko: ["그림", "일러스트", "만화", "작품"], en: ["art", "illustration", "drawing", "painting", "cartoon"], ja: ["イラスト", "絵", "アート", "漫画"], es: ["arte", "ilustración", "dibujo", "pintura"] } },
  { key: "map", ids: ["map"], ocr: [],
    label: { ko: "지도", en: "Maps", ja: "地図", es: "Mapas" },
    words: { ko: ["지도"], en: ["map", "maps"], ja: ["地図", "マップ"], es: ["mapa", "mapas"] } },
  { key: "receipt", ids: ["receipt", "ticket", "money", "credit_card", "gift_card"],
    // Vision often calls paper-ish things (packets, flyers) "ticket"/"receipt": trust the picture
    // alone only when it is very sure; otherwise the text has to back it up.
    pictureMinConf: 0.6,
    ocr: ["영수증", "결제금액", "합계금액", "승인번호", "receipt"],
    label: { ko: "영수증·티켓", en: "Receipts & Tickets", ja: "レシート・チケット", es: "Recibos y entradas" },
    words: { ko: ["영수증", "티켓", "표", "돈", "카드"], en: ["receipt", "ticket", "money", "card"], ja: ["レシート", "チケット", "お金"], es: ["recibo", "entrada", "billete", "dinero"] } },
];

const OBJ_CAT_BY_KEY = Object.fromEntries(OBJECT_CATEGORIES.map((c) => [c.key, c]));
const objSquash = (s) => s.normalize("NFC").toLowerCase().replace(/\s+/g, "");

/** Search words → categories. Exact word, or a prefix of a word once the query is long enough. */
function categoriesForQuery(q) {
  const sq = objSquash(q);
  if (!sq) return [];
  const minPrefix = /[ㄱ-힣぀-ヿ一-鿿]/.test(sq) ? 2 : 3; // CJK vs Latin
  return OBJECT_CATEGORIES.filter((c) =>
    Object.values(c.words).flat().concat(Object.values(c.label)).some((w) => {
      const sw = objSquash(w);
      return sw === sq || (sq.length >= minPrefix && sw.startsWith(sq));
    }));
}

/** Vision labels below this confidence are too often wrong to search by (stored ones go down to 0.2). */
const OBJECT_SEARCH_MIN_CONF = 0.35;

const isLatin = (w) => /^[\x00-\x7F]+$/.test(w);
const escRe = (w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
/** Latin words must match as whole words ("ade" must not match "made"); Korean/Japanese as substrings. */
function textHasWord(text, word) {
  if (isLatin(word)) return new RegExp(`\\b${escRe(word)}\\b`, "i").test(text);
  return objSquash(text).includes(objSquash(word));
}

/**
 * Category keys present in a screenshot.
 * - With an on-device model description (`ai`), its categories decide what the picture shows —
 *   it looks at the photo itself and ignores app chrome, so it is far more reliable than labels.
 * - Without it, confident Vision labels are used.
 * - Either way, tell-tale words in the text (e.g. "아메리카노") add categories.
 */
function objectCategoriesOf(objects, text, ai) {
  const conf = new Map((objects || []).map((o) => [o.id, o.conf]));
  const t = text || "";
  const aiCats = new Set(ai?.categories || []);
  return OBJECT_CATEGORIES.filter((c) => {
    const fromPicture = ai
      ? aiCats.has(c.key)
      : c.ids.some((id) => (conf.get(id) ?? 0) >= (c.pictureMinConf ?? OBJECT_SEARCH_MIN_CONF));
    return fromPicture || c.ocr.some((w) => textHasWord(t, w));
  }).map((c) => c.key);
}

const objCatLabel = (key, lang) => OBJ_CAT_BY_KEY[key]?.label[lang] || OBJ_CAT_BY_KEY[key]?.label.en || key;
