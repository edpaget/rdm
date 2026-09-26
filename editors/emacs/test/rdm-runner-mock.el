;;; rdm-runner-mock.el --- A counting mock of rdm-run for the worktree and modeline tests  -*- lexical-binding: t; -*-

;;; Commentary:

;; `rdm-mock-with-runner' replaces `rdm-run' (the one function every rdm
;; process goes through, `rdm-run-sync' and `rdm-call' included) with a
;; mock that records each call's argv and directory and answers from a
;; reply function, and `rdm-context' with a fixed context, so no `rdm
;; info' can run.  Not itself a `*-test.el'; required by the test files.

;;; Code:

(require 'cl-lib)
(require 'rdm-core)
(require 'rdm-view)

(defvar rdm-mock-defer nil
  "Non-nil: the mock queues each reply in `rdm-mock--queue' instead of
delivering it, as a real process would before it exits.")

(defvar rdm-mock--queue nil
  "Queued replies, oldest first, each a thunk delivering one.")

(defun rdm-mock-deliver ()
  "Deliver the oldest queued reply; return non-nil if there was one."
  (when-let* ((thunk (pop rdm-mock--queue)))
    (funcall thunk)
    t))

(defmacro rdm-mock-with-runner (reply &rest body)
  "Run BODY with `rdm-run' and `rdm-context' mocked.
REPLY is called with the argv (without the program) and the directory
rdm would run in, and returns the result: a parsed JSON value or text,
or (:mock-error . MESSAGE) to fail.  Binds `calls' to the list of
\(ARGV . DIRECTORY), oldest first, and `contexts' to the
`default-directory' of each `rdm-context' call, newest first."
  (declare (indent 1) (debug t))
  `(let ((calls nil) (contexts nil) (reply-fn ,reply)
         (rdm-mock-defer nil) (rdm-mock--queue nil)
         (rdm-item--lookup (make-hash-table :test #'equal)))
     (cl-letf (((symbol-function 'rdm-context)
                (lambda ()
                  (push default-directory contexts)
                  (list :root "/plan" :project "p" :rdm-bin "rdm"
                        :default-branch "main")))
               ((symbol-function 'rdm-run)
                (cl-function
                 (lambda (command &key directory callback on-error &allow-other-keys)
                   (let* ((argv (cdr command))
                          (result (funcall reply-fn argv directory))
                          (deliver
                           (lambda ()
                             (if (eq (car-safe result) :mock-error)
                                 (funcall (or on-error #'ignore)
                                          (list 'user-error (cdr result)))
                               (funcall (or callback #'ignore) result)))))
                     (setq calls (append calls (list (cons argv directory))))
                     (if rdm-mock-defer
                         (setq rdm-mock--queue (append rdm-mock--queue (list deliver)))
                       (funcall deliver)))
                   nil))))
       ,@body)))

(defun rdm-mock-argvs (calls)
  "The argvs of CALLS, oldest first."
  (mapcar #'car calls))

(provide 'rdm-runner-mock)
;;; rdm-runner-mock.el ends here
