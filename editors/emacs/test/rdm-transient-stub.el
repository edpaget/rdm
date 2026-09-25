;;; rdm-transient-stub.el --- The logging rdm stub for write-surface tests  -*- lexical-binding: t; -*-

;;; Commentary:

;; An `rdm' shell stub that logs each invocation's `RDM_SESSION' and argv
;; (and a `create''s stdin), with the fixtures the write-surface tests
;; share.  Required by rdm-transient-test.el and rdm-compose-test.el; not
;; itself a `*-test.el', so the suite driver loads it only through them.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'seq)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-test-helpers)

(defconst rdm-transient-test--statuses
  "\"not-started\",\"in-progress\",\"done\",\"blocked\",\"wont-fix\""
  "The status values the stub's `describe' declares, as JSON list items.")

(defun rdm-transient-test--stub (log root &rest opts)
  "An rdm stub logging to LOG and resolving ROOT, shaped by OPTS.
OPTS: `:tags' (the JSON tag array every `show' reports, default
\"[\\\"bug\\\"]\"), `:extra' (JSON list items appended to the declared
statuses) and `:refuse' (a line every update prints on stderr before
exiting 1).  Each call appends RDM_SESSION and the argv, tab-separated;
a `create' also saves its stdin to LOG.stdin (see
`rdm-transient-test--stdin')."
  (rdm-test-stub
   (format "{ printf '%%s' \"$RDM_SESSION\"; for a in \"$@\"; do printf '\\t%%s' \"$a\"; done; printf '\\n'; } >> '%s'
case \"$1 $2\" in
  'info '*) printf '{\"root\":\"%s\",\"project\":\"p\",\"default_branch\":\"main\"}\\n' ;;
  'describe '*) printf '{\"name\":\"%%s\",\"fields\":[{\"name\":\"title\",\"enum_values\":[]},{\"name\":\"status\",\"enum_values\":[%s%s]}]}\\n' \"$2\" ;;
  *' show') printf '{\"slug\":\"t1\",\"title\":\"T\",\"status\":\"open\",\"tags\":%s,\"body\":\"B\"}\\n' ;;
  *' update')
    if [ -n '%s' ]; then echo '%s' >&2; exit 1; fi
    echo \"Updated $1 '$3'\" ;;
  *' create')
    cat > '%s.stdin'
    if [ \"$1\" = phase ]; then echo \"Created phase 'phase-9-$3' in roadmap\"
    else echo \"Created $1 '$3'\"; fi ;;
  'status '*) echo 'Uncommitted changes:' ;;
  'commit '*) echo 'Committed 1 file(s).' ;;
  'discard '*) echo 'Discarded.' ;;
  *) echo \"unexpected $*\" >&2; exit 9 ;;
esac"
           log root rdm-transient-test--statuses
           (or (plist-get opts :extra) "")
           (or (plist-get opts :tags) "[\"bug\"]")
           (or (plist-get opts :refuse) "")
           (or (plist-get opts :refuse) "")
           log)))

(defun rdm-transient-test--stdin (log)
  "What the stub's last `create' read on stdin, or nil."
  (let ((file (concat log ".stdin")))
    (when (file-exists-p file)
      (with-temp-buffer
        (insert-file-contents file)
        (buffer-string)))))

(defun rdm-transient-test--calls (log)
  "The logged calls in LOG, oldest first, each (SESSION . ARGV)."
  (when (file-exists-p log)
    (with-temp-buffer
      (insert-file-contents log)
      (mapcar (lambda (line) (let ((parts (split-string line "\t")))
                               (cons (car parts) (cdr parts))))
              (split-string (buffer-string) "\n" t)))))

(defun rdm-transient-test--argvs (log)
  "The argvs in LOG other than `info', oldest first."
  (seq-remove (lambda (argv) (equal (car argv) "info"))
              (mapcar #'cdr (rdm-transient-test--calls log))))

(defmacro rdm-transient-test--with-stub (opts &rest body)
  "Run BODY against an rdm stub shaped by OPTS (see `rdm-transient-test--stub').
Binds `log'.  The environment inherits a foreign RDM_SESSION, and
`rdm-session' is nil; detail buffers are killed afterwards."
  (declare (indent 1) (debug t))
  `(let* ((default-directory (file-name-as-directory (make-temp-file "rdm-cwd" t)))
          (root (make-temp-file "rdm-root" t))
          (log (make-temp-file "rdm-log"))
          (stub (apply #'rdm-transient-test--stub log root ,opts))
          (rdm-session nil)
          (rdm-item--lookup (make-hash-table :test #'equal)))
     (rdm-test-with-env (list (concat "RDM_BIN=" stub)
                              "RDM_SESSION=inherited-other")
       (unwind-protect (progn ,@body)
         (dolist (b (buffer-list))
           (when (eq (buffer-local-value 'major-mode b) 'rdm-view-mode)
             (kill-buffer b)))))))

(defconst rdm-transient-test--phase
  '(:kind phase :roadmap "r1" :stem "phase-2-two"))

(defconst rdm-transient-test--task '(:kind task :slug "t1"))

(defun rdm-transient-test--refuse-prompts ()
  "Fail on any prompt: the code under test must not read input here."
  (error "Unexpected prompt"))

(provide 'rdm-transient-stub)
;;; rdm-transient-stub.el ends here
