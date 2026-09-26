;;; rdm-package-test.el --- ERT tests for the installed rdm package  -*- lexical-binding: t; -*-

;;; Commentary:

;; Installs the package the way docs/emacs.md's straight recipe does
;; (every editors/emacs/*.el except rdm-dev.el), generates its autoloads
;; as a package manager would, and loads it in a child `emacs -Q --batch'
;; whose `load-path' holds only that directory, so no soft dependency is
;; reachable.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'subr-x)
(require 'rdm-test-helpers)

(defconst rdm-package-test--package-dir
  (file-name-as-directory
   (expand-file-name ".." (file-name-directory
                           (or load-file-name buffer-file-name))))
  "The package source directory.")

(defun rdm-package-test--install ()
  "Copy the recipe's file set into a fresh directory; return the directory."
  (let ((dir (file-name-as-directory (make-temp-file "rdm-recipe" t))))
    (dolist (file (directory-files rdm-package-test--package-dir nil "\\.el\\'"))
      (unless (equal file "rdm-dev.el")
        (copy-file (expand-file-name file rdm-package-test--package-dir)
                   (expand-file-name file dir))))
    dir))

(defun rdm-package-test--child (dir loaddefs form)
  "Evaluate FORM in `emacs -Q --batch' with only DIR on `load-path'.
LOADDEFS is loaded first.  The child's environment has no `RDM_'
variable; `RDM_BIN' names a stub that fails every call, so nothing
resolves and no real rdm can run.  Returns what FORM printed, read."
  (let* ((stub (rdm-test-stub "echo 'no plan here' >&2; exit 1"))
         (process-environment (cons (concat "RDM_BIN=" stub)
                                    (rdm-test-clean-environment)))
         (default-directory (file-name-as-directory (make-temp-file "rdm-cwd" t)))
         (emacs (expand-file-name invocation-name invocation-directory))
         (program `(progn
                     (setq load-path (cons ,dir load-path))
                     (load ,loaddefs nil t)
                     (prin1 ,form))))
    (with-temp-buffer
      (let* ((stderr (make-temp-file "rdm-child-stderr"))
             (code (call-process emacs nil (list t stderr) nil "-Q" "--batch"
                                 "--eval" (prin1-to-string program))))
        (unless (eq code 0)
          (ert-fail (list "child emacs failed" code (buffer-string)
                          (with-temp-buffer
                            (insert-file-contents stderr)
                            (buffer-string)))))
        (goto-char (point-min))
        (read (current-buffer))))))

(ert-deftest rdm-package/recipe-loads-from-autoloads ()
  "The recipe's file set loads under `emacs -Q' from generated autoloads.
It excludes rdm-dev.el, `(require \\='rdm)' works, and an autoloaded
command runs without rdm-dev or any soft dependency."
  (let* ((dir (rdm-package-test--install))
         (loaddefs (expand-file-name "rdm-autoloads.el"
                                     (make-temp-file "rdm-loaddefs" t))))
    (should-not (file-exists-p (expand-file-name "rdm-dev.el" dir)))
    (let ((inhibit-message t))
      (loaddefs-generate dir loaddefs))
    (let ((got (rdm-package-test--child
                dir loaddefs
                '(let ((autoloaded (autoloadp (symbol-function 'rdm-context-reset))))
                   (rdm-context-reset)
                   (require 'rdm)
                   (list :autoloaded autoloaded
                         :ran (fboundp 'rdm-context)
                         :rdm (featurep 'rdm)
                         :rdm-dev (featurep 'rdm-dev)
                         :soft (delq nil
                                     (mapcar (lambda (f) (and (featurep f) f))
                                             '(evil consult embark marginalia
                                                    magit markdown-mode
                                                    nano-modeline))))))))
      (should (eq (plist-get got :autoloaded) t))
      (should (plist-get got :ran))
      (should (plist-get got :rdm))
      (should-not (plist-get got :rdm-dev))
      (should-not (plist-get got :soft)))))

(provide 'rdm-package-test)
;;; rdm-package-test.el ends here
