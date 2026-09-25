;;; rdm-test-helpers.el --- Shared ERT helpers for the rdm package  -*- lexical-binding: t; -*-

;;; Commentary:

;; Fake `rdm' stubs, an isolated environment macro, and the soft-dependency
;; helper every test file reuses.  Loaded by the suite driver (run.el) and
;; required by each `*-test.el'.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'rdm-core)

(defun rdm-test-stub (body)
  "Create an executable `rdm' shell stub running BODY; return its path.
The stub lives in a fresh temporary directory."
  (let* ((dir (make-temp-file "rdm-stub" t))
         (file (expand-file-name "rdm" dir)))
    (with-temp-file file
      (insert "#!/bin/sh\n" body "\n"))
    (set-file-modes file #o755)
    file))

(defun rdm-test-clean-environment ()
  "`process-environment' without any `RDM_' variable."
  (cl-remove-if (lambda (e) (string-prefix-p "RDM_" e)) process-environment))

(defmacro rdm-test-with-env (env &rest body)
  "Run BODY with a clean rdm context: no `RDM_*' variables except ENV.
ENV is a list of \"VAR=value\" strings.  The overrides `rdm-root',
`rdm-project' and `rdm-bin' are nil and the context cache is empty,
both before and after BODY."
  (declare (indent 1) (debug t))
  `(let ((process-environment (append ,env (rdm-test-clean-environment)))
         (rdm-root nil) (rdm-project nil) (rdm-bin nil))
     (rdm-context-reset)
     (unwind-protect (progn ,@body)
       (rdm-context-reset))))

(defun rdm-test-info-json (&rest fields)
  "JSON text for a canned `rdm info' reply from plist FIELDS."
  (json-serialize (or fields '(:default_branch "main"))))

(defmacro rdm-test-with-real (&rest body)
  "Run BODY against the binary and fixture the suite driver supplies.
Binds `bin', `root' and `project'; `default-directory' is the plan repo.
The environment keeps RDM_SESSION and drops every other `RDM_' variable.
Skips the test unless the suite driver supplies RDM_TEST_BIN,
RDM_TEST_ROOT and RDM_TEST_PROJECT."
  (declare (indent 0) (debug t))
  `(let ((bin (getenv "RDM_TEST_BIN"))
         (root (getenv "RDM_TEST_ROOT"))
         (project (getenv "RDM_TEST_PROJECT"))
         (session (getenv "RDM_SESSION")))
     (skip-unless (and bin root project))
     (let ((default-directory (file-name-as-directory root)))
       (rdm-test-with-env (when session (list (concat "RDM_SESSION=" session)))
         ,@body))))

(defmacro rdm-test-with-soft-dep (feature &rest body)
  "Run BODY when FEATURE is loaded, else skip the test.
Tag the test with FEATURE's name so the suite can tell a skip in the
bare pass from a run in the init-directory pass."
  (declare (indent 1) (debug t))
  `(progn
     (skip-unless (featurep ,feature))
     ,@body))

(provide 'rdm-test-helpers)
;;; rdm-test-helpers.el ends here
