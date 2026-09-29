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
;;   x    the `rdm-dispatch' transient (status, tags, body, reviews, create,
;;        commit)
;;   K    describe the review comment at point
;;   ]c   next review comment
;;   [c   previous review comment
;;   gA   mark the review comment at point addressed or won't-fix
;;   C    comment on the whole document in your draft review
;;   gS   submit your draft review with a verdict (confirmed)
;;
;; and in visual state
;;
;;   c    comment on the selected body lines in your draft review (the
;;        lines the selection touches are the comment's anchor)
;;
;; The first comment starts a draft when you have none, and the buffer
;; then shows the item as of the draft's `created_commit'.
;;
;; `rdm-compose-mode' (rdm-compose.el) starts in insert state; in normal
;; state `ZZ' submits the buffer and `ZQ' aborts it, as in evil-collection's
;; `git-commit' bindings.
;;
;; In a commit message buffer (`rdm-git-commit-mode', rdm-git.el), `gd'
;; in normal state inserts a `Done:' trailer, like vanilla `C-c C-x d'.
;;
;; Later phases add their evil bindings here.

;;; Code:

(require 'rdm-view)
(require 'rdm-transient)
(require 'rdm-compose)
(require 'rdm-git)
(require 'rdm-review)

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
    (kbd "x") #'rdm-dispatch
    (kbd "K") #'rdm-review-comment-at-point
    (kbd "]c") #'rdm-review-next-comment
    (kbd "[c") #'rdm-review-previous-comment
    (kbd "gA") #'rdm-review-resolve
    (kbd "C") #'rdm-review-comment-document
    (kbd "gS") #'rdm-review-submit)
  (evil-define-key* 'visual rdm-view-mode-map
    (kbd "c") #'rdm-review-comment-region)
  (evil-set-initial-state 'rdm-compose-mode 'insert)
  (evil-define-key* 'normal rdm-compose-mode-map
    (kbd "ZZ") #'rdm-compose-submit
    (kbd "ZQ") #'rdm-compose-abort)
  (evil-define-key* 'normal rdm-git-commit-mode-map
    (kbd rdm-git-commit-evil-key) #'rdm-insert-done-line))

(with-eval-after-load 'evil
  (rdm-evil-setup))

(provide 'rdm-evil)
;;; rdm-evil.el ends here
