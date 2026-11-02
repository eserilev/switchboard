-- Switchboard: a file next to its old side, in diff mode (SPEC 11.14).
-- The hub writes this file to its nvim folder and calls `diff` through luaeval.
local M = {}

-- `tree` is the review's folder, `base` the commit of the old side, `path` the
-- file in the folder, `line` the line to show.
function M.diff(tree, base, path, line)
  -- nvim is the worktree's own and stays open: close the old side of an earlier
  -- call, so the splits never pile up.
  for _, w in ipairs(vim.api.nvim_list_wins()) do
    if vim.b[vim.api.nvim_win_get_buf(w)].sb_old then
      pcall(vim.api.nvim_win_close, w, true)
    end
  end
  vim.cmd('diffoff!')
  vim.cmd('edit ' .. vim.fn.fnameescape(tree .. '/' .. path))
  local cur = vim.api.nvim_get_current_win()
  local ft = vim.bo.filetype
  local old = vim.fn.systemlist({ 'git', '-C', tree, 'show', base .. ':' .. path })
  -- A new file has no old side.
  if vim.v.shell_error ~= 0 then old = {} end
  vim.cmd('leftabove vnew')
  local b = vim.api.nvim_get_current_buf()
  vim.api.nvim_buf_set_lines(b, 0, -1, false, old)
  vim.bo[b].buftype = 'nofile'
  vim.bo[b].bufhidden = 'wipe'
  vim.bo[b].swapfile = false
  vim.bo[b].modifiable = false
  vim.bo[b].filetype = ft
  vim.b[b].sb_old = true
  pcall(vim.api.nvim_buf_set_name, b, 'sb://' .. base:sub(1, 8) .. '/' .. path)
  vim.cmd('diffthis')
  vim.api.nvim_set_current_win(cur)
  vim.cmd('diffthis')
  pcall(vim.api.nvim_win_set_cursor, cur, { math.max(line, 1), 0 })
  vim.cmd('normal! zz')
  return 1
end

return M
