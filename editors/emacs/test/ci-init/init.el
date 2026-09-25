;;; init.el --- Soft-dependency init directory for the rdm Emacs suite  -*- lexical-binding: t; -*-

;;; Commentary:

;; Provides the optional packages the rdm package integrates with (evil,
;; magit, consult, embark) so the suite's second pass can run the tests
;; tagged for them.  Copy this file into an empty directory DIR and run
;;
;;   emacs --batch --init-directory DIR -l DIR/init.el
;;
;; once to install the packages into DIR/elpa (needs network).  Afterwards,
;; point RDM_EMACS_INIT_DIR at DIR; the suite loads this file again, which
;; then only activates and loads what is installed.

;;; Code:

(require 'cl-lib)
(require 'package)

(setq package-user-dir (expand-file-name "elpa" user-emacs-directory))
(setq package-archives
      '(("gnu" . "https://elpa.gnu.org/packages/")
        ("nongnu" . "https://elpa.nongnu.org/nongnu/")
        ("melpa" . "https://melpa.org/packages/")))
(package-initialize)

(defconst rdm-ci-init-packages '(evil magit consult embark)
  "The soft dependencies the suite's second pass provides.")

(let ((missing (cl-remove-if #'package-installed-p rdm-ci-init-packages)))
  (when missing
    (package-refresh-contents)
    (dolist (pkg missing)
      (package-install pkg))))

(dolist (pkg rdm-ci-init-packages)
  (require pkg))

;;; init.el ends here
