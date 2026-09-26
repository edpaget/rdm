;;; rdm-dev.el --- Development helpers for the rdm Emacs package  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; Development only: not autoloaded and not required by any shipped file.
;; Load it by hand (M-x load-file) while hacking on the package, then run
;; `rdm-dev-reload' after editing to pick up changed `defvar' / `defcustom'
;; forms, which a plain `load' leaves at their old values.

;;; Code:

(require 'cl-lib)

(declare-function rdm-context-reset "rdm-core")

(defvar rdm-dev-after-reload-hook nil
  "Hook run after `rdm-dev-reload' has reloaded the package.")

(defconst rdm-dev--directory
  (file-name-directory (or load-file-name buffer-file-name))
  "The directory holding the package's source files.")

(defun rdm-dev--files ()
  "The package's source files, `rdm-core.el' first, excluding this one."
  (let* ((files (directory-files rdm-dev--directory t
                                "\\`rdm\\(?:-.*\\)?\\.el\\'"))
         (files (cl-remove-if
                 (lambda (f) (equal (file-name-nondirectory f) "rdm-dev.el"))
                 files))
         (core (cl-find "rdm-core.el" files
                        :key #'file-name-nondirectory :test #'equal)))
    (if core (cons core (delete core files)) files)))

(defun rdm-dev--defined-variables (file)
  "Symbols that top-level `defvar'/`defcustom'/`defconst' forms in FILE define."
  (with-temp-buffer
    (insert-file-contents file)
    (goto-char (point-min))
    (let (syms form)
      (while (setq form (condition-case nil
                            (read (current-buffer))
                          (end-of-file nil)))
        (when (and (consp form)
                   (memq (car form) '(defvar defcustom defconst defvar-local))
                   (symbolp (cadr form))
                   ;; A bare (defvar foo) declares, it does not initialize.
                   (cddr form))
          (push (cadr form) syms)))
      (nreverse syms))))

(defun rdm-dev-reload ()
  "Reload the rdm package from source, re-initializing its variables.
Every variable a package file defines is unbound first, so changed
`defvar' / `defcustom' initial values take effect; the context cache is
then cleared and `rdm-dev-after-reload-hook' runs."
  (interactive)
  (dolist (file (rdm-dev--files))
    (with-temp-buffer
      (dolist (sym (rdm-dev--defined-variables file))
        (makunbound sym)))
    (load file nil t t))
  (when (fboundp 'rdm-context-reset)
    (rdm-context-reset))
  (run-hooks 'rdm-dev-after-reload-hook)
  (message "rdm: reloaded %d files" (length (rdm-dev--files))))

(provide 'rdm-dev)
;;; rdm-dev.el ends here
