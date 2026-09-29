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

;;;; Doctor

(defun rdm-doctor--info-value (info key)
  "KEY of INFO, the parsed `rdm info' reply, or nil when absent or failed."
  (and (not (eq (car-safe info) 'failed))
       (rdm--nonempty (plist-get info key))))

(defun rdm-doctor--binary (bin info)
  "BIN, a located (PATH . RUNG) or nil, reported against INFO, as text.
The path is the one `rdm info' reports for itself when INFO has it,
else the located command, marked as unconfirmed by `rdm info'."
  (let ((reported (rdm-doctor--info-value info :binary)))
    (cond ((null bin) "not found: set `rdm-bin', RDM_BIN, or put rdm on PATH")
          (reported (format "%s (located via %s)" reported (cdr bin)))
          (t (format "%s (from %s; rdm info unavailable)" (car bin) (cdr bin))))))

(defun rdm-doctor--mise ()
  "Whether this buffer's environment came from mise.el, as report text."
  (cond ((bound-and-true-p mise-mode)
         "yes: `mise-mode' is on in this buffer, so its RDM_* variables \
come from .mise.toml")
        ((featurep 'mise) "no: mise.el is loaded, but `mise-mode' is off in this buffer")
        (t "no: mise.el is not loaded")))

(defun rdm-doctor--setting (setting info key)
  "SETTING, a (VALUE . RUNG) or nil, else KEY of INFO, as report text.
INFO is the parsed `rdm info' reply, or nil."
  (let ((from-info (rdm-doctor--info-value info key)))
    (cond (setting (format "%s (from %s)" (car setting) (cdr setting)))
          (from-info (format "%s (from rdm info)" from-info))
          (t "unset"))))

(defun rdm-doctor-report ()
  "The `rdm-doctor' report for the current buffer, as a string.
Never signals: a failure to resolve is reported."
  (let* ((dir (rdm--project-dir))
         (bin (rdm--locate-bin))
         (root (rdm--local-setting 'root))
         (project (rdm--local-setting 'project))
         (info (if bin
                   (condition-case e
                       (rdm--info (car bin) dir (car root) (car project))
                     (error (cons 'failed (error-message-string e))))
                 '(failed . "not run: no rdm binary")))
         (context (condition-case e
                      (let ((ctx (rdm-context)))
                        (format "resolved (root %s, project %s)"
                                (plist-get ctx :root) (plist-get ctx :project)))
                    (error (concat "unresolved: " (error-message-string e))))))
    (concat
     (format "rdm doctor for %s\n\n" (buffer-name))
     (format "Directory: %s\n" (abbreviate-file-name dir))
     (format "Binary:    %s\n" (rdm-doctor--binary bin info))
     (format "Version:   %s\n" (or (rdm-doctor--info-value info :version)
                                   "unknown"))
     (format "Root:      %s\n" (rdm-doctor--setting root info :root))
     (format "Project:   %s\n" (rdm-doctor--setting project info :project))
     (format "Branch:    %s\n" (rdm-doctor--setting nil info :default_branch))
     (format "rdm info:  %s\n" (if (eq (car-safe info) 'failed)
                                   (if (string-prefix-p "not run" (cdr info))
                                       (cdr info)
                                     (concat "failed: " (cdr info)))
                                 "ok"))
     (format "mise.el:   %s\n" (rdm-doctor--mise))
     (format "Context:   %s\n" context))))

;;;###autoload
(defun rdm-doctor ()
  "Report how rdm resolves for the current buffer, in *rdm-doctor*.
Shows the `rdm' binary as `rdm info' reports its own path, with the
rung that located it (`rdm-bin', RDM_BIN or PATH), its version as
`rdm info' reports it (\"unknown\" when `rdm info' fails or does not
run), the plan repo root and project and the rung that supplied each
\(`rdm-root' / `rdm-project', the buffer's environment, or `rdm info'),
the default branch, whether the environment came from mise.el, and
whether `rdm-context' resolves.  Returns the report buffer."
  (interactive)
  (let ((report (rdm-doctor-report))
        (buf (get-buffer-create "*rdm-doctor*")))
    (with-current-buffer buf
      (special-mode)
      (let ((inhibit-read-only t))
        (erase-buffer)
        (insert report)
        (goto-char (point-min))))
    (display-buffer buf)
    buf))

(provide 'rdm)
;;; rdm.el ends here
