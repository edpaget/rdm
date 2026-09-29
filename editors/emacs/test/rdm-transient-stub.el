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

(defconst rdm-transient-test--changes
  "{\"user\":[{\"path\":\"projects/p/tasks/t1.md\",\"change\":\"modified\"}],\
\"others\":[{\"path\":\"projects/p/tasks/o.md\",\"change\":\"added\"}],\"unattributed\":[]}"
  "The `changes' object the stub's status, commit and discard report.")

(defun rdm-transient-test--stub (log root &rest opts)
  "An rdm stub logging to LOG and resolving ROOT, shaped by OPTS.
OPTS: `:tags' (the JSON tag array every `show' reports, default
\"[\\\"bug\\\"]\"), `:extra' (JSON list items appended to the declared
statuses), `:terminal' (the JSON `terminal_values' of every status
field, default \"[\\\"done\\\",\\\"wont-fix\\\"]\"), `:flags' (its JSON
`value_flags', default blocked → reason) and `:refuse' (a line every
update prints on stderr before exiting 1).  Each call appends
RDM_SESSION and the argv, tab-separated; a `create' also saves its
stdin to LOG.stdin (see `rdm-transient-test--stdin') and prints the
created item in its `show' shape."
  (rdm-test-stub
   (format "{ printf '%%s' \"$RDM_SESSION\"; for a in \"$@\"; do printf '\\t%%s' \"$a\"; done; printf '\\n'; } >> '%s'
roadmap=
for a in \"$@\"; do case \"$a\" in --roadmap=*) roadmap=\"${a#--roadmap=}\" ;; esac; done
case \"$1 $2\" in
  'info '*) printf '{\"root\":\"%s\",\"project\":\"p\",\"default_branch\":\"main\"}\\n' ;;
  'describe '*) printf '{\"name\":\"%%s\",\"fields\":[{\"name\":\"title\",\"enum_values\":[]},{\"name\":\"status\",\"enum_values\":[%s%s],\"terminal_values\":%s,\"value_flags\":%s}]}\\n' \"$2\" ;;
  'roadmap show') printf '{\"kind\":\"roadmap\",\"slug\":\"%%s\",\"ref\":\"%%s\",\"review_ref\":\"roadmap/%%s\",\"title\":\"T\",\"tags\":%s,\"body\":\"B\"}\\n' \"$3\" \"$3\" \"$3\" ;;
  'phase show') printf '{\"kind\":\"phase\",\"stem\":\"%%s\",\"roadmap\":\"%%s\",\"ref\":\"%%s/%%s\",\"review_ref\":\"phase/%%s/%%s\",\"title\":\"T\",\"status\":\"open\",\"tags\":%s,\"body\":\"B\"}\\n' \"$3\" \"$roadmap\" \"$roadmap\" \"$3\" \"$roadmap\" \"$3\" ;;
  'task show') printf '{\"kind\":\"task\",\"slug\":\"%%s\",\"ref\":\"task/%%s\",\"review_ref\":\"task/%%s\",\"title\":\"T\",\"status\":\"open\",\"tags\":%s,\"body\":\"B\"}\\n' \"$3\" \"$3\" \"$3\" ;;
  *' update')
    if [ -n '%s' ]; then echo '%s' >&2; exit 1; fi
    echo \"Updated $1 '$3'\" ;;
  *' create')
    cat > '%s.stdin'
    case \"$1\" in
      roadmap) printf '{\"kind\":\"roadmap\",\"slug\":\"%%s\",\"ref\":\"%%s\",\"review_ref\":\"roadmap/%%s\"}\\n' \"$3\" \"$3\" \"$3\" ;;
      phase) printf '{\"kind\":\"phase\",\"stem\":\"phase-9-%%s\",\"roadmap\":\"%%s\",\"ref\":\"%%s/phase-9-%%s\",\"review_ref\":\"phase/%%s/phase-9-%%s\"}\\n' \"$3\" \"$roadmap\" \"$roadmap\" \"$3\" \"$roadmap\" \"$3\" ;;
      *) printf '{\"kind\":\"task\",\"slug\":\"%%s\",\"ref\":\"task/%%s\",\"review_ref\":\"task/%%s\"}\\n' \"$3\" \"$3\" \"$3\" ;;
    esac ;;
  'review list') echo '[]' ;;
  'status '*) printf '%%s\\n' '{\"changes\":%s,\"merge\":null,\"sync\":null}' ;;
  'commit '*) printf '%%s\\n' '{\"commit\":\"0123456789abcdef0123\",\"changeset\":\"s\",\"skipped\":[],\"advisory\":null,\"changes\":%s}' ;;
  'discard '*) printf '%%s\\n' '{\"merge_aborted\":false,\"restored\":[\"projects/p/tasks/t1.md\"],\"skipped\":[],\"changes\":%s}' ;;
  *) echo \"unexpected $*\" >&2; exit 9 ;;
esac"
           log root rdm-transient-test--statuses
           (or (plist-get opts :extra) "")
           (or (plist-get opts :terminal) "[\"done\",\"wont-fix\"]")
           (or (plist-get opts :flags) "[{\"value\":\"blocked\",\"flag\":\"reason\"}]")
           (or (plist-get opts :tags) "[\"bug\"]")
           (or (plist-get opts :tags) "[\"bug\"]")
           (or (plist-get opts :tags) "[\"bug\"]")
           (or (plist-get opts :refuse) "")
           (or (plist-get opts :refuse) "")
           log
           rdm-transient-test--changes
           rdm-transient-test--changes
           rdm-transient-test--changes)))

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
  "The argvs in LOG other than `info', `describe' and `review list'.
Oldest first.  Every render of a detail buffer also reads its reviews
\(rdm-review.el), and a status write reads rdm's declared status
metadata; those reads are not what the write-surface tests are about."
  (seq-remove (lambda (argv) (or (member (car argv) '("info" "describe"))
                                 (equal (seq-take argv 2) '("review" "list"))))
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
  '(:kind phase :roadmap "r1" :stem "phase-2-two" :ref "r1/phase-2-two"))

(defconst rdm-transient-test--task '(:kind task :slug "t1" :ref "task/t1"))

(defun rdm-transient-test--refuse-prompts ()
  "Fail on any prompt: the code under test must not read input here."
  (error "Unexpected prompt"))

(provide 'rdm-transient-stub)
;;; rdm-transient-stub.el ends here
