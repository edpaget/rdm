;;; rdm-evil.el --- Evil bindings for the rdm Emacs integration  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; Evil-first bindings for rdm buffers, following evil-collection's
;; conventions.  Nothing here requires evil: the bindings are installed
;; once evil loads, and without it the `special-mode' keys (`g', `q',
;; `RET') of `rdm-view-mode' keep working.
;;
;; `rdm-view-mode' starts in normal state, where `j'/`k'/`gg'/`G' move as
;; usual and
;;
;;   RET  open the item at point (a roadmap view's phase row)
;;   gr   refresh (re-fetch from rdm)
;;   q    quit the window
;;   y    copy the identifier of the item at point
;;   gd   copy the item's `Done:' commit line
;;   x    the `rdm-dispatch' transient (status, tags, body, create, commit)
;;
;; `rdm-compose-mode' (rdm-compose.el) starts in insert state; in normal
;; state `ZZ' submits the buffer and `ZQ' aborts it, as in evil-collection's
;; `git-commit' bindings.
;;
;; Later phases add their evil bindings here.

;;; Code:

(require 'rdm-view)
(require 'rdm-transient)
(require 'rdm-compose)

(declare-function evil-set-initial-state "ext:evil-core" (mode state))
(declare-function evil-define-key* "ext:evil-core" (state keymap key def &rest bindings))

(defun rdm-evil-setup ()
  "Install rdm's evil bindings.  Runs once evil is loaded."
  (evil-set-initial-state 'rdm-view-mode 'normal)
  (evil-define-key* 'normal rdm-view-mode-map
    (kbd "RET") #'rdm-view-open-at-point
    (kbd "gr") #'revert-buffer
    (kbd "q") #'quit-window
    (kbd "y") #'rdm-view-copy-identifier
    (kbd "gd") #'rdm-view-copy-done-line
    (kbd "x") #'rdm-dispatch)
  (evil-set-initial-state 'rdm-compose-mode 'insert)
  (evil-define-key* 'normal rdm-compose-mode-map
    (kbd "ZZ") #'rdm-compose-submit
    (kbd "ZQ") #'rdm-compose-abort))

(with-eval-after-load 'evil
  (rdm-evil-setup))

(provide 'rdm-evil)
;;; rdm-evil.el ends here
