if vim.b.did_ftplugin then
  return
end
vim.b.did_ftplugin = true
vim.bo.commentstring = "// %s"
vim.treesitter.start()
vim.b.undo_ftplugin = "setlocal commentstring< | lua vim.treesitter.stop()"
