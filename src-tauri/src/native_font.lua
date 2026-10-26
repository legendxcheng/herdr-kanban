-- Evaluated as a CLI override AFTER WezTerm loads the user's normal config.
-- Keep its path, relative resources, reload watching, mouse and non-font keys.
local config, journal, preference = ...
local wezterm = require 'wezterm'
local default_size = config.font_size or 12.0
local function valid(value)
  return type(value) == 'number' and value == value and value >= 6 and value <= 72
end
local file = io.open(preference, 'rb')
if file then
  local text = file:read(1024)
  file:close()
  local ok, saved = pcall(wezterm.json_parse, text or '')
  if ok and type(saved) == 'table' and valid(saved.font_size) then
    config.font_size = saved.font_size
  end
end

local sizes = {}
-- A retained viewer may regain focus after another machine changed the shared size.
wezterm.on('window-focus-changed', function(window)
  if not window:is_focused() then return end
  local source = io.open(preference, 'rb')
  if not source then return end
  local text = source:read(1024); source:close()
  local ok, saved = pcall(wezterm.json_parse, text or '')
  if not ok or type(saved) ~= 'table' then return end
  local size = valid(saved.font_size) and saved.font_size or default_size
  sizes[window:window_id()] = size
  if math.abs(window:effective_config().font_size - size) > 0.001 then
    local overrides = window:get_config_overrides() or {}
    overrides.font_size = size
    overrides.adjust_window_size_when_changing_font_size = false
    window:set_config_overrides(overrides)
  end
end)
local function change(window, direction)
  local id = window:window_id()
  local size = sizes[id] or window:effective_config().font_size
  if direction == 0 then
    size = default_size
  else
    size = math.max(6, math.min(72, size * (direction > 0 and 1.1 or 1 / 1.1)))
  end
  -- Append complete records, never truncate a record the host might be reading.
  -- Only the current view's journal can update the shared preference.
  local output, err = io.open(journal, 'ab')
  if output then
    local written, write_err = output:write(direction == 0 and 'null\n' or string.format('%.10f\n', size))
    local closed, close_err = output:close()
    if not written or not closed then
      wezterm.log_error('Kanban font preference: ' .. tostring(write_err or close_err))
    end
  else
    wezterm.log_error('Kanban font preference: ' .. tostring(err))
  end
  sizes[id] = size
  local overrides = window:get_config_overrides() or {}
  overrides.font_size = size
  -- The host owns the window bounds; changing font changes rows/columns only.
  overrides.adjust_window_size_when_changing_font_size = false
  window:set_config_overrides(overrides)
end

local events = {
  IncreaseFontSize = 'kanban-font-increase',
  DecreaseFontSize = 'kanban-font-decrease',
  ResetFontSize = 'kanban-font-reset',
}
wezterm.on(events.IncreaseFontSize, function(window) change(window, 1) end)
wezterm.on(events.DecreaseFontSize, function(window) change(window, -1) end)
wezterm.on(events.ResetFontSize, function(window) change(window, 0) end)

local function replace(action)
  if type(action) == 'string' and events[action] then
    return wezterm.action.EmitEvent(events[action])
  end
  if type(action) == 'table' and action.Multiple then
    for index, child in ipairs(action.Multiple) do
      action.Multiple[index] = replace(child)
    end
  end
  return action
end
local keys = {}
if not config.disable_default_key_bindings then
  for _, key in ipairs(wezterm.gui.default_keys()) do
    if type(key.action) == 'string' and events[key.action] then
      key.action = replace(key.action)
      table.insert(keys, key)
    end
  end
  -- Numpad add/subtract are not bound by all WezTerm versions.
  table.insert(keys, { key = 'Add', mods = 'CTRL', action = wezterm.action.EmitEvent(events.IncreaseFontSize) })
  table.insert(keys, { key = 'Subtract', mods = 'CTRL', action = wezterm.action.EmitEvent(events.DecreaseFontSize) })
end
-- User bindings come last and retain their precedence over defaults.
for _, key in ipairs(config.keys or {}) do
  key.action = replace(key.action)
  table.insert(keys, key)
end
config.keys = keys
for _, bindings in pairs(config.key_tables or {}) do
  for _, key in ipairs(bindings) do key.action = replace(key.action) end
end
return 'NONE'
