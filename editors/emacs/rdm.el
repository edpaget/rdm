;;; rdm.el --- Roadmaps, phases and tasks from an rdm plan repo  -*- lexical-binding: t; -*-

;; Author: Edward Paget <ed.paget@gmail.com>
;; URL: https://github.com/edpaget/rdm
;; Version: 0.1.0
;; Package-Requires: ((emacs "31.1"))
;; Keywords: tools, convenience

;; This file is not part of GNU Emacs.

;;; Commentary:

;; The package entry for the rdm Emacs integration: browse, pick, edit,
;; review and commit an rdm plan repo's roadmaps, phases and tasks from
;; Emacs, driving the `rdm' CLI.  Installation (the straight recipe), the
;; environment contract, the command inventory and the keymaps are in
;; docs/emacs.md in the rdm repository.
;;
;; Requiring this file loads only the UI-free core (rdm-core.el).  Every
;; user-facing command is autoloaded from its own file, so a package
;; manager's generated autoloads are all a configuration needs.  Optional
;; packages (evil, consult, embark, marginalia, magit, markdown-mode,
;; nano-modeline) are used when present and never required; transient is
;; built into Emacs.  rdm-dev.el is a development helper, not part of the
;; package.

;;; Code:

(require 'rdm-core)

(provide 'rdm)
;;; rdm.el ends here
