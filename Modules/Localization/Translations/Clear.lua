---@class QuestieTraceCore
local Core = QuestieTraceCore

---@type l10n
local l10n = Core.l10n

local clearLocales = {
  ["Removed %s shared session(s)."] = {
    ["enUS"] = true,
    ["deDE"] = "%s geteilte Sitzung(en) entfernt.",
    ["esES"] = "Se eliminaron %s sesión(es) compartida(s).",
    ["esMX"] = "Se eliminaron %s sesión(es) compartida(s).",
    ["frFR"] = "%s session(s) partagée(s) supprimée(s).",
    ["koKR"] = "공유된 세션 %s개를 제거했습니다.",
    ["ptBR"] = "%s sessão(ões) compartilhada(s) removida(s).",
    ["ruRU"] = "Удалено переданных сессий: %s.",
    ["zhCN"] = "已移除 %s 个已分享的会话。",
    ["zhTW"] = "已移除 %s 個已分享的工作階段。",
  },
  ["No shared sessions to remove."] = {
    ["enUS"] = true,
    ["deDE"] = "Keine geteilten Sitzungen zum Entfernen.",
    ["esES"] = "No hay sesiones compartidas para eliminar.",
    ["esMX"] = "No hay sesiones compartidas para eliminar.",
    ["frFR"] = "Aucune session partagée à supprimer.",
    ["koKR"] = "제거할 공유된 세션이 없습니다.",
    ["ptBR"] = "Nenhuma sessão compartilhada para remover.",
    ["ruRU"] = "Нет переданных сессий для удаления.",
    ["zhCN"] = "没有可移除的已分享会话。",
    ["zhTW"] = "沒有可移除的已分享工作階段。",
  },
  ["Delete ALL %s collected session(s), including data you have not shared yet? This cannot be undone."] = {
    ["enUS"] = true,
    ["deDE"] = "ALLE %s gesammelten Sitzung(en) löschen, einschließlich noch nicht geteilter Daten? Dies kann nicht rückgängig gemacht werden.",
    ["esES"] = "¿Eliminar TODAS las %s sesión(es) recopilada(s), incluidos los datos que aún no has compartido? Esto no se puede deshacer.",
    ["esMX"] = "¿Eliminar TODAS las %s sesión(es) recopilada(s), incluidos los datos que aún no has compartido? Esto no se puede deshacer.",
    ["frFR"] = "Supprimer TOUTES les %s session(s) collectée(s), y compris les données que vous n'avez pas encore partagées ? Cette action est irréversible.",
    ["koKR"] = "아직 공유하지 않은 데이터를 포함하여 수집된 세션 %s개를 모두 삭제하시겠습니까? 이 작업은 되돌릴 수 없습니다.",
    ["ptBR"] = "Excluir TODAS as %s sessão(ões) coletada(s), incluindo dados que você ainda não compartilhou? Isso não pode ser desfeito.",
    ["ruRU"] = "Удалить ВСЕ собранные сессии (%s), включая ещё не переданные данные? Это действие необратимо.",
    ["zhCN"] = "要删除全部 %s 个已收集的会话吗（包括尚未分享的数据）？此操作无法撤销。",
    ["zhTW"] = "要刪除全部 %s 個已收集的工作階段嗎（包含尚未分享的資料）？此操作無法復原。",
  },
  ["Deleted %s session(s)."] = {
    ["enUS"] = true,
    ["deDE"] = "%s Sitzung(en) gelöscht.",
    ["esES"] = "Se eliminaron %s sesión(es).",
    ["esMX"] = "Se eliminaron %s sesión(es).",
    ["frFR"] = "%s session(s) supprimée(s).",
    ["koKR"] = "세션 %s개를 삭제했습니다.",
    ["ptBR"] = "%s sessão(ões) excluída(s).",
    ["ruRU"] = "Удалено сессий: %s.",
    ["zhCN"] = "已删除 %s 个会话。",
    ["zhTW"] = "已刪除 %s 個工作階段。",
  },
}

for k, v in pairs(clearLocales) do
  l10n.translations[k] = v
end
