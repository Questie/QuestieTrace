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
  ["Copy the text below and submit it at the URL above. Please do this again each time this window has new data to share, since one submission only covers what happened up to that point. This helps us build the Questie database. Player and guild names are never included."] = {
    ["enUS"] = true,
    ["deDE"] = "Kopieren Sie den Text unten und senden Sie ihn über die obige URL. Bitte wiederholen Sie dies jedes Mal, wenn dieses Fenster neue Daten enthält, da eine Übermittlung nur abdeckt, was bis zu diesem Zeitpunkt passiert ist. Das hilft uns, die Questie-Datenbank aufzubauen. Spielernamen und Gildennamen sind niemals enthalten.",
    ["esES"] = "Copia el texto de abajo y envíalo a la URL de arriba. Repite esto cada vez que esta ventana tenga nuevos datos que compartir, ya que un envío solo cubre lo ocurrido hasta ese momento. Esto nos ayuda a construir la base de datos de Questie. Los nombres de jugadores y hermandades nunca se incluyen.",
    ["esMX"] = "Copia el texto de abajo y envíalo a la URL de arriba. Repite esto cada vez que esta ventana tenga nuevos datos que compartir, ya que un envío solo cubre lo ocurrido hasta ese momento. Esto nos ayuda a construir la base de datos de Questie. Los nombres de jugadores y hermandades nunca se incluyen.",
    ["frFR"] = "Copiez le texte ci-dessous et soumettez-le à l'URL ci-dessus. Recommencez chaque fois que cette fenêtre contient de nouvelles données à partager, car une soumission ne couvre que ce qui s'est passé jusqu'à ce moment-là. Cela nous aide à construire la base de données Questie. Les noms des joueurs et des guildes ne sont jamais inclus.",
    ["koKR"] = "아래 텍스트를 복사하여 위의 URL에 제출해 주세요. 이 창에 새로운 데이터가 있을 때마다 다시 제출해 주세요. 한 번의 제출은 그 시점까지의 데이터만 포함합니다. 이 데이터는 퀘스티 데이터베이스 구축에 도움이 됩니다. 플레이어 및 길드 이름은 절대 포함되지 않습니다.",
    ["ptBR"] = "Copie o texto abaixo e envie na URL acima. Repita isso sempre que esta janela tiver novos dados para compartilhar, pois cada envio cobre apenas o que aconteceu até aquele momento. Isso nos ajuda a construir o banco de dados do Questie. Nomes de jogadores e guildas nunca são incluídos.",
    ["ruRU"] = "Скопируйте текст ниже и отправьте его по ссылке выше. Повторяйте это каждый раз, когда в этом окне появляются новые данные, так как одна отправка охватывает только то, что произошло до этого момента. Это помогает нам строить базу данных Questie. Имена игроков и гильдий никогда не включаются.",
    ["zhCN"] = "复制下方文本并提交至上方 URL。每次此窗口出现新数据时都请重复此操作，因为一次提交只涵盖到那时为止发生的数据。这些数据帮助我们构建 Questie 数据库。绝不包含玩家和公会名称。",
    ["zhTW"] = "複製下方文字並提交至上方 URL。每次此視窗出現新資料時都請重複此操作，因為一次提交只涵蓋到那時為止發生的資料。這些資料協助我們建構 Questie 資料庫。絕不包含玩家和公會名稱。",
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
  ["Nothing new to share yet. Keep playing and check back later."] = {
    ["enUS"] = true,
    ["deDE"] = "Noch nichts Neues zum Teilen. Spiele weiter und schau später noch einmal vorbei.",
    ["esES"] = "Todavía no hay nada nuevo que compartir. Sigue jugando y vuelve más tarde.",
    ["esMX"] = "Todavía no hay nada nuevo que compartir. Sigue jugando y vuelve más tarde.",
    ["frFR"] = "Rien de nouveau à partager pour le moment. Continuez à jouer et revenez plus tard.",
    ["koKR"] = "아직 공유할 새 데이터가 없습니다. 계속 플레이하고 나중에 다시 확인해 주세요.",
    ["ptBR"] = "Ainda não há nada novo para compartilhar. Continue jogando e volte mais tarde.",
    ["ruRU"] = "Пока нечем поделиться. Продолжайте играть и загляните позже.",
    ["zhCN"] = "目前没有新的可分享内容。继续游戏，稍后再来看看。",
    ["zhTW"] = "目前沒有新的可分享內容。繼續遊玩，稍後再回來看看。",
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
