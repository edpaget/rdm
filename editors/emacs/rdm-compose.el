;;; rdm-compose.el --- The compose buffer for rdm bodies  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; `rdm-compose' opens a `*rdm compose*' buffer in `rdm-compose-mode'
;; (derived from `markdown-mode' when it is installed, else `text-mode')
;; for typing a body: an item's body on create or on "Edit body" in
;; `rdm-dispatch', and any other text a caller wants, such as a review
;; comment.  The caller supplies the function the text is submitted to.
;;
;;   C-c C-c  submit: call the submit function with the buffer's text and,
;;            when it returns, kill the buffer
;;   C-c C-k  abort: kill the buffer without submitting
;;
;; A submit function that signals leaves the buffer (and the text typed
;; into it) alive, so the user can fix the problem and submit again.
;; Each compose gets a buffer of its own, so an open one is never
;; overwritten.  Under evil (see rdm-evil.el) the buffer starts in insert
;; state and, in normal state, `ZZ' submits and `ZQ' aborts.

;;; Code:

(require 'subr-x)
(require 'rdm-core)
(require 'rdm-view)

(defalias 'rdm-compose--parent-mode
  (if (require 'markdown-mode nil t) 'markdown-mode 'text-mode)
  "The major mode `rdm-compose-mode' derives from.")

(defvar-local rdm-compose--submit nil
  "The function this compose buffer's text is submitted to.")

(defvar-keymap rdm-compose-mode-map
  :doc "Keymap for `rdm-compose-mode'."
  "C-c C-c" #'rdm-compose-submit
  "C-c C-k" #'rdm-compose-abort)

(define-derived-mode rdm-compose-mode rdm-compose--parent-mode "rdm-compose"
  "Major mode for composing an rdm body.
\\<rdm-compose-mode-map>\\[rdm-compose-submit] submits the buffer's text; \
\\[rdm-compose-abort] aborts.
\\{rdm-compose-mode-map}"
  ;; The evil bindings (rdm-evil.el requires this file, so not at top level).
  (require 'rdm-evil))

(defun rdm-compose (text submit description)
  "Pop up a compose buffer holding TEXT; return the buffer.
SUBMIT is called with the buffer's text (a string, unchanged) when the
user submits with `rdm-compose-submit', with the compose buffer
current; it runs rdm in the context of the buffer `rdm-compose' was
called from, which the compose buffer inherits.  When SUBMIT returns,
the buffer is killed; when it signals, the buffer is kept and the error
propagates.  `rdm-compose-abort' kills the buffer without calling
SUBMIT.  DESCRIPTION is a short text shown in the header line, such as
\"Body of task fix-x\".

This is the stable entry point for callers composing text for rdm."
  (let ((source (current-buffer))
        (buf (generate-new-buffer "*rdm compose*")))
    (with-current-buffer buf
      (rdm-compose-mode)
      (rdm-view--inherit-context source)
      (setq rdm-compose--submit submit)
      (setq header-line-format
            (concat description "  "
                    (substitute-command-keys
                     "\\<rdm-compose-mode-map>\\[rdm-compose-submit] submit, \
\\[rdm-compose-abort] abort")))
      (insert (or text ""))
      (goto-char (point-min))
      (set-buffer-modified-p nil))
    (pop-to-buffer buf)
    buf))

(defun rdm-compose--close (buf)
  "Kill the compose buffer BUF, restoring the window that showed it."
  (let ((win (get-buffer-window buf)))
    (if win
        (quit-window t win)
      (kill-buffer buf))))

(defun rdm-compose--check ()
  "Signal a `user-error' unless this is a compose buffer."
  (unless (derived-mode-p 'rdm-compose-mode)
    (user-error "rdm: not an rdm compose buffer")))

(defun rdm-compose-submit ()
  "Submit this buffer's text to its submit function, then kill the buffer.
If the submit function signals, the buffer is kept."
  (interactive)
  (rdm-compose--check)
  (let ((buf (current-buffer)))
    (funcall rdm-compose--submit
             (buffer-substring-no-properties (point-min) (point-max)))
    (when (buffer-live-p buf)
      (rdm-compose--close buf))))

(defun rdm-compose-abort ()
  "Kill this compose buffer without submitting it."
  (interactive)
  (rdm-compose--check)
  (rdm-compose--close (current-buffer))
  (message "rdm: compose aborted"))

(provide 'rdm-compose)
;;; rdm-compose.el ends here
