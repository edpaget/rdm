;;; run.el --- Batch driver for the rdm Emacs package suite  -*- lexical-binding: t; -*-

;;; Commentary:

;; Invoked by the `emacs_package' nextest binary as
;;
;;   emacs -Q --batch -l editors/emacs/test/run.el
;;   emacs -q --init-directory DIR --batch -l DIR/init.el -l .../run.el
;;
;; 1. Byte-compiles every editors/emacs/*.el with warnings as errors, writing
;;    the .elc files to a temporary directory (the source tree stays clean).
;; 2. Runs checkdoc over the same files; any diagnostic fails the pass.
;; 3. Loads every test/*-test.el and runs the ERT suite.
;; 4. Writes one JSON record per test (name, tags, result) to the file named
;;    by RDM_EMACS_RESULTS, when set.
;;
;; Exits non-zero on a compile failure, a checkdoc diagnostic or any
;; unexpected test result.

;;; Code:

(require 'bytecomp)
(require 'ert)
(require 'cl-lib)

(defconst rdm-run--test-dir
  (file-name-directory (or load-file-name buffer-file-name)))

(defconst rdm-run--package-dir
  (file-name-as-directory (expand-file-name ".." rdm-run--test-dir)))

(add-to-list 'load-path rdm-run--package-dir)
(add-to-list 'load-path rdm-run--test-dir)

(defun rdm-run--compile ()
  "Byte-compile the package into a temp dir; return non-nil on success."
  (let* ((out (make-temp-file "rdm-elc" t))
         (byte-compile-error-on-warn t)
         (byte-compile-dest-file-function
          (lambda (src)
            (expand-file-name (concat (file-name-nondirectory src) "c") out)))
         (ok t))
    (dolist (file (directory-files rdm-run--package-dir t "\\.el\\'"))
      (unless (condition-case err
                  (byte-compile-file file)
                (error (message "rdm: compiling %s: %S" file err) nil))
        (message "rdm: byte-compile failed: %s" file)
        (setq ok nil)))
    (delete-directory out t)
    ok))

(defun rdm-run--checkdoc ()
  "Run checkdoc over the package; return non-nil when it reports nothing.
Every diagnostic is printed as FILE:LINE: MESSAGE."
  (require 'checkdoc)
  (let ((ok t))
    (dolist (file (directory-files rdm-run--package-dir t "\\.el\\'"))
      (let ((buf (find-file-noselect file))
            (checkdoc-autofix-flag 'never)
            (checkdoc-spellcheck-documentation-flag nil))
        (unwind-protect
            (cl-letf (((symbol-function 'checkdoc-error)
                       (lambda (point msg)
                         (setq ok nil)
                         (message "rdm: checkdoc: %s:%d: %s"
                                  (file-name-nondirectory file)
                                  (with-current-buffer buf
                                    (line-number-at-pos (or point (point-min))))
                                  msg))))
              (with-current-buffer buf
                (checkdoc-current-buffer t)))
          (kill-buffer buf))))
    ok))

(defun rdm-run--result-name (result)
  "The result kind of RESULT as a string."
  (cond ((ert-test-passed-p result) "passed")
        ((ert-test-skipped-p result) "skipped")
        ((ert-test-failed-p result) "failed")
        (t "other")))

(defun rdm-run--write-results (stats file)
  "Write one record per test in STATS to FILE as a JSON array."
  (let ((records
         (cl-loop for test across (ert--stats-tests stats)
                  for result across (ert--stats-test-results stats)
                  collect
                  (list :name (symbol-name (ert-test-name test))
                        :tags (vconcat (mapcar #'symbol-name
                                               (ert-test-tags test)))
                        :result (rdm-run--result-name result)
                        :expected (if (ert-test-result-expected-p test result)
                                      t :false)))))
    (with-temp-file file
      (insert (json-serialize (vconcat records))))))

(unless (rdm-run--compile)
  (kill-emacs 2))

(unless (rdm-run--checkdoc)
  (kill-emacs 3))

(dolist (file (directory-files rdm-run--test-dir t "-test\\.el\\'"))
  (load file nil t))

(let* ((stats (ert-run-tests-batch t))
       (results (getenv "RDM_EMACS_RESULTS")))
  (when (and results (not (string-empty-p results)))
    (rdm-run--write-results stats results))
  (kill-emacs (if (zerop (ert-stats-completed-unexpected stats)) 0 1)))

;;; run.el ends here
