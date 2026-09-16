---@class QuestieTraceCore
local Core = QuestieTraceCore

---@type l10n
local l10n = Core.l10n

local exportReminderLocales = {
  ["It is time to share your trace data. %s"] = {
    ["enUS"] = true,
    ["deDE"] = "Es ist Zeit, deine Trace-Daten zu teilen. %s",
    ["esES"] = "Es hora de compartir tus datos. %s",
    ["esMX"] = "Es hora de compartir tus datos. %s",
    ["frFR"] = "Il est temps de partager vos données. %s",
    ["itIT"] = "È il momento di condividere i tuoi dati di tracciamento. %s",
    ["koKR"] = "추적 데이터를 공유할 시간입니다. %s",
    ["ptBR"] = "É hora de compartilhar seus dados. %s",
    ["ruRU"] = "Пора поделиться данными трассировки. %s",
    ["zhCN"] = "是时候分享你的追踪数据了。%s",
    ["zhTW"] = "是時候分享你的追蹤資料了。%s",
  },
  ["Click here to open the export window"] = {
    ["enUS"] = true,
    ["deDE"] = "Klicken Sie hier, um das Exportfenster zu öffnen",
    ["esES"] = "Haz clic aquí para abrir la ventana de exportación",
    ["esMX"] = "Haz clic aquí para abrir la ventana de exportación",
    ["frFR"] = "Cliquez ici pour ouvrir la fenêtre d'exportation",
    ["itIT"] = "Fai clic qui per aprire la finestra di esportazione",
    ["koKR"] = "내보내기 창을 열려면 여기를 클릭하세요",
    ["ptBR"] = "Clique aqui para abrir a janela de exportação",
    ["ruRU"] = "Нажмите здесь, чтобы открыть окно экспорта",
    ["zhCN"] = "点击此处打开导出窗口",
    ["zhTW"] = "點擊此處開啟匯出視窗",
  },
}

for k, v in pairs(exportReminderLocales) do
  l10n.translations[k] = v
end
