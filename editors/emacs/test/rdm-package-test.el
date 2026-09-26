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
It excludes rdm-dev.el, and the autoloaded `rdm-doctor' and
`rdm-context-reset' run, loading `rdm' without rdm-dev or any soft
dependency.  The stub binary resolves nothing, which the doctor reports."
  (let* ((dir (rdm-package-test--install))
         (loaddefs (expand-file-name "rdm-autoloads.el"
                                     (make-temp-file "rdm-loaddefs" t))))
    (should-not (file-exists-p (expand-file-name "rdm-dev.el" dir)))
    (let ((inhibit-message t))
      (loaddefs-generate dir loaddefs))
    (let ((got (rdm-package-test--child
                dir loaddefs
                '(let ((autoloaded (list (autoloadp (symbol-function 'rdm-context-reset))
                                         (autoloadp (symbol-function 'rdm-doctor))))
                       (report (let ((inhibit-message t))
                                 (with-current-buffer (rdm-doctor)
                                   (buffer-string)))))
                   (rdm-context-reset)
                   (list :autoloaded autoloaded
                         :ran (and (string-match-p "^Context: +unresolved" report)
                                   t)
                         :rdm (featurep 'rdm)
                         :rdm-dev (featurep 'rdm-dev)
                         :soft (delq nil
                                     (mapcar (lambda (f) (and (featurep f) f))
                                             '(evil consult embark marginalia
                                                    magit markdown-mode
                                                    nano-modeline))))))))
      (should (equal (plist-get got :autoloaded) '(t t)))
      (should (plist-get got :ran))
      (should (plist-get got :rdm))
      (should-not (plist-get got :rdm-dev))
      (should-not (plist-get got :soft)))))

(provide 'rdm-package-test)
;;; rdm-package-test.el ends here
