---@class QuestieTraceCore
local Core = QuestieTraceCore

---@type l10n
local l10n = Core.l10n

local exportUILocales = {
  ["QuestieTrace Export"] = {
    ["enUS"] = true,
    ["deDE"] = "QuestieTrace Export",
    ["esES"] = "Exportar QuestieTrace",
    ["esMX"] = "Exportar QuestieTrace",
    ["frFR"] = "Export QuestieTrace",
    ["koKR"] = "QuestieTrace 내보내기",
    ["ptBR"] = "Exportar QuestieTrace",
    ["ruRU"] = "Экспорт QuestieTrace",
    ["zhCN"] = "QuestieTrace 导出",
    ["zhTW"] = "QuestieTrace 匯出",
  },
  ["Copy the text below and submit it at the URL above. This data helps us build the Questie database. Player and guild names are never included."] = {
    ["enUS"] = true,
    ["deDE"] = "Kopieren Sie den Text unten und senden Sie ihn über die obige URL. Diese Daten helfen uns, die Questie-Datenbank aufzubauen. Spielernamen und Gildennamen sind niemals enthalten.",
    ["esES"] = "Copie el texto de abajo y envíelo en la URL de arriba. Estos datos nos ayudan a construir la base de datos de Questie. Los nombres de jugadores y hermandades nunca se incluyen.",
    ["esMX"] = "Copie el texto de abajo y envíelo en la URL de arriba. Estos datos nos ayudan a construir la base de datos de Questie. Los nombres de jugadores y hermandades nunca se incluyen.",
    ["frFR"] = "Copiez le texte ci-dessous et soumettez-le à l'URL ci-dessus. Ces données nous aident à construire la base de données Questie. Les noms des joueurs et des guildes ne sont jamais inclus.",
    ["koKR"] = "아래 텍스트를 복사하여 위의 URL에 제출해 주세요. 이 데이터는 퀘스티 데이터베이스 구축에 도움이 됩니다. 플레이어 및 길드 이름은 절대 포함되지 않습니다.",
    ["ptBR"] = "Copie o texto abaixo e envie na URL acima. Estes dados nos ajudam a construir o banco de dados do Questie. Nomes de jogadores e guildas nunca são incluídos.",
    ["ruRU"] = "Скопируйте текст ниже и отправьте его по ссылке выше. Эти данные помогают нам строить базу данных Questie. Имена игроков и гильдий никогда не включаются.",
    ["zhCN"] = "复制下方文本并提交至上方 URL。这些数据帮助我们构建 Questie 数据库。绝不包含玩家和公会名称。",
    ["zhTW"] = "複製下方文字並提交至上方 URL。這些資料協助我們建構 Questie 資料庫。絕不包含玩家和公會名稱。",
  },
  ["Close"] = {
    ["enUS"] = true,
    ["deDE"] = "Schließen",
    ["esES"] = "Cerrar",
    ["esMX"] = "Cerrar",
    ["frFR"] = "Fermer",
    ["koKR"] = "닫기",
    ["ptBR"] = "Fechar",
    ["ruRU"] = "Закрыть",
    ["zhCN"] = "关闭",
    ["zhTW"] = "關閉",
  },
  ["Nothing to export yet. Start a capture first."] = {
    ["enUS"] = true,
    ["deDE"] = "Es gibt noch nichts zu exportieren. Starte zuerst eine Aufzeichnung.",
    ["esES"] = "Todavía no hay nada que exportar. Inicia primero una captura.",
    ["esMX"] = "Todavía no hay nada que exportar. Inicia primero una captura.",
    ["frFR"] = "Rien à exporter pour le moment. Démarrez d'abord une capture.",
    ["koKR"] = "아직 내보낼 데이터가 없습니다. 먼저 캡처를 시작하세요.",
    ["ptBR"] = "Ainda não há nada para exportar. Inicie uma captura primeiro.",
    ["ruRU"] = "Пока нечего экспортировать. Сначала начните запись.",
    ["zhCN"] = "目前没有可导出的内容。请先开始捕获。",
    ["zhTW"] = "目前沒有可匯出的內容。請先開始擷取。",
  },
  ["ERROR: Client does not have required codec support (C_EncodingUtil, Enum.CompressionMethod, LibDeflate)"] = {
    ["enUS"] = true,
    ["deDE"] = "FEHLER: Der Client verfügt nicht über die erforderliche Codec-Unterstützung (C_EncodingUtil, Enum.CompressionMethod, LibDeflate)",
    ["esES"] = "ERROR: El cliente no tiene el soporte de códec necesario (C_EncodingUtil, Enum.CompressionMethod, LibDeflate)",
    ["esMX"] = "ERROR: El cliente no tiene el soporte de códec necesario (C_EncodingUtil, Enum.CompressionMethod, LibDeflate)",
    ["frFR"] = "ERREUR : Le client ne dispose pas du support de codec requis (C_EncodingUtil, Enum.CompressionMethod, LibDeflate)",
    ["koKR"] = "오류: 클라이언트에 필요한 코덱 지원이 없습니다 (C_EncodingUtil, Enum.CompressionMethod, LibDeflate)",
    ["ptBR"] = "ERRO: O cliente não possui o suporte de codec necessário (C_EncodingUtil, Enum.CompressionMethod, LibDeflate)",
    ["ruRU"] = "ОШИБКА: клиент не поддерживает необходимые кодеки (C_EncodingUtil, Enum.CompressionMethod, LibDeflate)",
    ["zhCN"] = "错误：客户端缺少所需的编解码支持 (C_EncodingUtil, Enum.CompressionMethod, LibDeflate)",
    ["zhTW"] = "錯誤：用戶端缺少所需的編解碼支援 (C_EncodingUtil, Enum.CompressionMethod, LibDeflate)",
  },
}

for k, v in pairs(exportUILocales) do
  l10n.translations[k] = v
end
