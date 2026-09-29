;;; rdm-doctor-test.el --- ERT tests for rdm-doctor  -*- lexical-binding: t; -*-

;;; Commentary:

;; `rdm-doctor' against shell stubs (a context that cannot resolve, no
;; binary at all) and, in the `real-' tests, against the binary under
;; test and the seeded fixture the suite driver supplies.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'subr-x)
(require 'rdm)
(require 'rdm-test-helpers)

(defmacro rdm-doctor-test--in-temp-dir (&rest body)
  "Run BODY with `default-directory' a fresh non-project directory."
  (declare (indent 0) (debug t))
  `(let ((default-directory (file-name-as-directory (make-temp-file "rdm-cwd" t))))
     ,@body))

(defun rdm-doctor-test--line (report label)
  "The value on REPORT's line starting with LABEL and a colon, or nil."
  (when (string-match (concat "^" (regexp-quote label) ": *\\(.*\\)$") report)
    (match-string 1 report)))

(defun rdm-doctor-test--logging-stub (log body)
  "A stub that appends its arguments to LOG, then runs BODY."
  (rdm-test-stub (format "echo \"$*\" >> %s\n%s" (shell-quote-argument log) body)))

(defun rdm-doctor-test--calls (log)
  "The argument lines the logging stub wrote to LOG."
  (and (file-exists-p log)
       (split-string (with-temp-buffer
                       (insert-file-contents log)
                       (buffer-string))
                     "\n" t)))

(ert-deftest rdm-doctor/version-and-binary-come-from-info ()
  "Version and Binary are what `rdm info' reports; nothing runs --version."
  (rdm-doctor-test--in-temp-dir
    (let* ((log (make-temp-file "rdm-doctor-calls"))
           (stub (rdm-doctor-test--logging-stub
                  log
                  (format "echo '%s'"
                          (rdm-test-info-json :default_branch "main"
                                              :version "9.9.9"
                                              :binary "/opt/rdm/bin/rdm")))))
      (rdm-test-with-env (list (concat "RDM_BIN=" stub))
        (let ((report (rdm-doctor-report)))
          (should (equal (rdm-doctor-test--line report "Binary")
                         "/opt/rdm/bin/rdm (located via RDM_BIN)"))
          (should (equal (rdm-doctor-test--line report "Version") "9.9.9"))
          (should (equal (rdm-doctor-test--line report "rdm info") "ok"))
          (let ((calls (rdm-doctor-test--calls log)))
            (should (member "info --format json" calls))
            (should-not (seq-some (lambda (c) (string-match-p "--version\\|-V" c))
                                  calls))))))))

(ert-deftest rdm-doctor/unresolved-context-is-reported ()
  "A binary whose every call fails yields a report, not an error."
  (rdm-doctor-test--in-temp-dir
    (let* ((log (make-temp-file "rdm-doctor-calls"))
           (stub (rdm-doctor-test--logging-stub
                  log "echo 'no plan here' >&2; exit 1")))
      (rdm-test-with-env (list (concat "RDM_BIN=" stub))
        (let ((report (rdm-doctor-report)))
          (should (equal (rdm-doctor-test--line report "Binary")
                         (format "%s (from RDM_BIN; rdm info unavailable)" stub)))
          (should (equal (rdm-doctor-test--line report "Version") "unknown"))
          (should-not (seq-some (lambda (c) (string-match-p "--version" c))
                                (rdm-doctor-test--calls log)))
          (should (equal (rdm-doctor-test--line report "Root") "unset"))
          (should (equal (rdm-doctor-test--line report "Project") "unset"))
          (should (string-match-p "\\`failed: .*no plan here"
                                  (rdm-doctor-test--line report "rdm info")))
          (should (string-match-p "\\`unresolved: .*\\.mise\\.toml"
                                  (rdm-doctor-test--line report "Context"))))))))

(ert-deftest rdm-doctor/no-binary-is-reported ()
  "With no binary on any rung the doctor reports it and the rest as unknown."
  (rdm-doctor-test--in-temp-dir
    (rdm-test-with-env nil
      (let* ((exec-path nil)
             (report (rdm-doctor-report)))
        (should (string-prefix-p "not found"
                                 (rdm-doctor-test--line report "Binary")))
        (should (equal (rdm-doctor-test--line report "Version") "unknown"))
        (should (string-prefix-p "not run"
                                 (rdm-doctor-test--line report "rdm info")))
        (should (string-prefix-p "unresolved:"
                                 (rdm-doctor-test--line report "Context")))))))

(ert-deftest rdm-doctor/overrides-name-their-rung ()
  "Buffer-local overrides are reported as the rung that supplied each value."
  (rdm-doctor-test--in-temp-dir
    (let ((stub (rdm-test-stub "exit 1"))
          (root (make-temp-file "rdm-root" t)))
      (rdm-test-with-env (list "RDM_PROJECT=from-env")
        (setq rdm-bin stub rdm-root root)
        (let ((report (rdm-doctor-report)))
          (should (equal (rdm-doctor-test--line report "Binary")
                         (format "%s (from rdm-bin; rdm info unavailable)" stub)))
          (should (equal (rdm-doctor-test--line report "Version") "unknown"))
          (should (equal (rdm-doctor-test--line report "Root")
                         (format "%s (from rdm-root)" root)))
          (should (equal (rdm-doctor-test--line report "Project")
                         "from-env (from RDM_PROJECT)")))))))

(ert-deftest rdm-doctor/mise-provenance ()
  "The mise.el line follows `mise-mode' in the current buffer."
  (rdm-doctor-test--in-temp-dir
    (rdm-test-with-env (list (concat "RDM_BIN=" (rdm-test-stub "exit 1")))
      (with-temp-buffer
        (setq-local mise-mode t)
        (should (string-prefix-p
                 "yes" (rdm-doctor-test--line (rdm-doctor-report) "mise.el"))))
      (with-temp-buffer
        (setq-local mise-mode nil)
        (should (string-prefix-p
                 "no" (rdm-doctor-test--line (rdm-doctor-report) "mise.el")))))))

(ert-deftest rdm-doctor/real-report ()
  "Against the real binary the report names binary, root, project and state."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project))
                  process-environment))
    (setq rdm-bin bin)
    (let* ((buf (let ((inhibit-message t)) (rdm-doctor)))
           (report (with-current-buffer buf (buffer-string))))
      (let ((binary (rdm-doctor-test--line report "Binary")))
        (should (string-suffix-p " (located via rdm-bin)" binary))
        (should (equal (file-truename (string-remove-suffix
                                       " (located via rdm-bin)" binary))
                       (file-truename bin))))
      (should (string-match-p "\\`[0-9]+\\.[0-9]+\\.[0-9]+"
                              (rdm-doctor-test--line report "Version")))
      (should (equal (rdm-doctor-test--line report "Root")
                     (format "%s (from RDM_ROOT)" root)))
      (should (equal (rdm-doctor-test--line report "Project")
                     (format "%s (from RDM_PROJECT)" project)))
      (should (equal (rdm-doctor-test--line report "Branch") "main (from rdm info)"))
      (should (equal (rdm-doctor-test--line report "rdm info") "ok"))
      (should (string-prefix-p "resolved"
                               (rdm-doctor-test--line report "Context"))))))

(ert-deftest rdm-doctor/real-info-rung ()
  "With no RDM_* and a global config, root and project come from rdm info."
  (rdm-test-with-real
    (let* ((xdg (make-temp-file "rdm-xdg" t))
           (process-environment (cons (concat "XDG_CONFIG_HOME=" xdg)
                                      process-environment)))
      (make-directory (expand-file-name "rdm" xdg))
      (with-temp-file (expand-file-name "rdm/config.toml" xdg)
        (insert (format "root = %S\n" root)))
      (setq rdm-bin bin)
      (let ((report (rdm-doctor-report)))
        (should (string-suffix-p "(from rdm info)"
                                 (rdm-doctor-test--line report "Root")))
        (should (equal (rdm-doctor-test--line report "Project")
                       (format "%s (from rdm info)" project)))
        (should (string-prefix-p "resolved"
                                 (rdm-doctor-test--line report "Context")))))))

(provide 'rdm-doctor-test)
;;; rdm-doctor-test.el ends here
