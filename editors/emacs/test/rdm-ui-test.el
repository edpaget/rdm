;;; rdm-ui-test.el --- ERT tests for the rdm read UI  -*- lexical-binding: t; -*-

;;; Commentary:

;; Pickers, detail buffers, embark and evil integration.  Most tests mock
;; the runner (`rdm-run-sync') and the context resolver, capturing every
;; argv; the `real-' tests run against the binary under test and the
;; seeded plan repo (see `rdm-test-with-real').  Tests tagged `evil',
;; `consult', `embark' or `markdown-mode' run only where that package is
;; loaded.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'subr-x)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-pick)
(require 'rdm-evil)
(require 'rdm-test-helpers)

(defvar embark-keymap-alist)
(defvar evil-state)
(defvar consult-async-input-throttle)
(defvar consult-async-input-debounce)
(defvar consult-async-min-input)
(declare-function evil-local-mode "ext:evil-core" (&optional arg))

;;;; Mocked runner

(defun rdm-ui-test--kill-views ()
  "Kill every rdm detail buffer."
  (dolist (b (buffer-list))
    (when (eq (buffer-local-value 'major-mode b) 'rdm-view-mode)
      (kill-buffer b))))

(defun rdm-ui-test--flag (argv flag)
  "The value of `--FLAG=value' in ARGV, or nil."
  (let ((prefix (concat "--" flag "=")))
    (cl-some (lambda (a) (and (string-prefix-p prefix a)
                              (substring a (length prefix))))
             argv)))

(defun rdm-ui-test--reply (argv)
  "A canned reply to the rdm ARGV, shaped like rdm's parsed JSON."
  (let ((at (rdm-ui-test--flag argv "at")))
    (pcase argv
      (`("roadmap" "list" . ,_)
       (vector (list :slug "r1" :ref "r1" :review_ref "roadmap/r1"
                     :title "Road One" :total_phases 2
                     :done_phases 1 :progress "1/2 done" :tags ["ui"])))
      (`("roadmap" "show" ,slug . ,_)
       (append (list :project "p" :slug slug :ref slug
                     :review_ref (concat "roadmap/" slug) :title "Road One"
                     :total_phases 2 :done_phases 1 :progress "1/2 done"
                     :phases (vector (list :number 1 :stem "phase-1-one"
                                           :ref (concat slug "/phase-1-one")
                                           :review_ref (format "phase/%s/phase-1-one" slug)
                                           :title "One" :status "done")
                                     (list :number 2 :stem "phase-2-two"
                                           :ref (concat slug "/phase-2-two")
                                           :review_ref (format "phase/%s/phase-2-two" slug)
                                           :title "Two" :status "in-progress"))
                     :tags ["ui"] :body "Roadmap body.\n")
               (when at (list :revision at))))
      (`("phase" "list" . ,_)
       (vector (list :number 1 :stem "phase-1-one" :ref "r1/phase-1-one"
                     :review_ref "phase/r1/phase-1-one" :title "One" :status "done")
               (list :number 2 :stem "phase-2-two" :ref "r1/phase-2-two"
                     :review_ref "phase/r1/phase-2-two" :title "Two"
                     :status "in-progress")))
      (`("phase" "show" ,stem . ,_)
       (let ((roadmap (rdm-ui-test--flag argv "roadmap")))
         (append (list :body (format "Body of %s.\n" stem) :phase 2
                       :roadmap roadmap :ref (format "%s/%s" roadmap stem)
                       :review_ref (format "phase/%s/%s" roadmap stem)
                       :stem stem :title "Two" :status "in-progress")
                 (when at (list :revision at)))))
      (`("task" "list" . ,_)
       (vector (list :slug "t1" :ref "task/t1" :review_ref "task/t1"
                     :title "Task One" :status "open"
                     :priority "medium" :created "2026-09-25" :tags ["bug"])))
      (`("task" "show" ,slug . ,_)
       (append (list :slug slug :ref (concat "task/" slug)
                     :review_ref (concat "task/" slug) :project "p"
                     :title "Task One" :status "open"
                     :priority "medium" :tags ["bug"]
                     :body (if at "Old task body.\n" "Task body.\n"))
               (when at (list :revision at))))
      (`("search" ,_ "--type=roadmap,phase,task" . ,_)
       (vector (list :kind "roadmap" :identifier "r1" :project "p"
                     :title "Road One" :snippet "Road" :score 10
                     :roadmap "r1" :ref "r1" :review_ref "roadmap/r1")
               (list :kind "phase" :identifier "r1/phase-2-two" :project "p"
                     :title "Two" :snippet "Two" :score 9 :status "in-progress"
                     :roadmap "r1" :stem "phase-2-two" :ref "r1/phase-2-two"
                     :review_ref "phase/r1/phase-2-two")
               (list :kind "task" :identifier "t1" :project "p"
                     :title "Task One" :snippet "Task" :score 8
                     :tags ["bug"] :status "open" :slug "t1" :ref "task/t1"
                     :review_ref "task/t1")))
      (`("next" . ,_)
       (pcase (rdm-ui-test--flag argv "roadmap")
         ("blocked"
          (list :result "blocked-on-dependencies" :roadmap "blocked"
                :unmet (vector "r0" (list :roadmap "r9"))))
         ("empty" (list :result "nothing" :roadmap "empty"))
         (_ (list :result "phase" :roadmap "r1" :number 2 :stem "phase-2-two"
                  :status "in-progress"))))
      (`("hook" "done-line" . ,_)
       (if-let* ((task (rdm-ui-test--flag argv "task")))
           (list :line (format "Done: task/%s" task) :kind "task" :task task)
         (list :line (format "Done: %s/%s" (rdm-ui-test--flag argv "roadmap")
                             (rdm-ui-test--flag argv "phase"))
               :kind "phase"
               :roadmap (rdm-ui-test--flag argv "roadmap")
               :phase (rdm-ui-test--flag argv "phase"))))
      (_ (error "Unexpected argv %S" argv)))))

(defmacro rdm-ui-test--mocked (&rest body)
  "Run BODY with the runner mocked by `rdm-ui-test--reply'.
Binds `calls' to the argvs run so far, most recent first.  The context
is fixed to project \"p\"; detail buffers are killed afterwards.  The
review layer's `review list' reads (rdm-review.el) answer no reviews
and are not recorded: these tests are about the item reads."
  (declare (indent 0) (debug t))
  `(let ((calls nil)
         (rdm-item--lookup (make-hash-table :test #'equal))
         (kill-ring nil))
     (cl-letf (((symbol-function 'rdm-context)
                (lambda () (list :root "/tmp" :project "p" :rdm-bin "rdm")))
               ((symbol-function 'rdm-run-sync)
                (lambda (command &rest _)
                  (if (equal (seq-take (cdr command) 2) '("review" "list"))
                      []
                    (push (cdr command) calls)
                    (rdm-ui-test--reply (cdr command))))))
       (unwind-protect (progn ,@body)
         (rdm-ui-test--kill-views)))))

(defmacro rdm-ui-test--answers (answers &rest body)
  "Run BODY with `completing-read' returning ANSWERS in turn.
The answers are plain strings, as a completion UI may return them."
  (declare (indent 1) (debug t))
  `(let ((queue (copy-sequence ,answers)))
     (cl-letf (((symbol-function 'completing-read)
                (lambda (&rest _) (substring-no-properties (pop queue)))))
       ,@body)))

(defun rdm-ui-test--show (&rest words)
  "The expected read argv for WORDS in project \"p\"."
  (append words '("--project" "p" "--format" "json")))

;;;; Pickers

(ert-deftest rdm-pick/roadmap-argv ()
  "The roadmap picker lists roadmaps and opens the pick with `roadmap show'."
  (rdm-ui-test--mocked
    (rdm-ui-test--answers '("r1")
      (rdm-roadmap))
    (should (equal (reverse calls)
                   (list (rdm-ui-test--show "roadmap" "list")
                         (rdm-ui-test--show "roadmap" "show" "r1"))))))

(ert-deftest rdm-pick/roadmap-drill-down ()
  "With a prefix, the roadmap picker drills down into the roadmap's phases."
  (rdm-ui-test--mocked
    (rdm-ui-test--answers '("r1" "phase-2-two")
      (rdm-roadmap t))
    (should (equal (reverse calls)
                   (list (rdm-ui-test--show "roadmap" "list")
                         (rdm-ui-test--show "phase" "list" "--roadmap=r1")
                         (rdm-ui-test--show "phase" "show" "phase-2-two"
                                            "--roadmap=r1"))))))

(ert-deftest rdm-pick/phase-argv ()
  "The phase picker opens the pick with `phase show STEM --roadmap=SLUG'."
  (rdm-ui-test--mocked
    (rdm-ui-test--answers '("phase-1-one")
      (rdm-phase "r1"))
    (should (equal (car calls)
                   (rdm-ui-test--show "phase" "show" "phase-1-one"
                                      "--roadmap=r1")))))

(ert-deftest rdm-pick/task-argv ()
  "The task picker lists open tasks (all with a prefix) and shows the pick."
  (rdm-ui-test--mocked
    (rdm-ui-test--answers '("t1")
      (rdm-task))
    (should (equal (reverse calls)
                   (list (rdm-ui-test--show "task" "list")
                         (rdm-ui-test--show "task" "show" "t1"))))
    (setq calls nil)
    (rdm-ui-test--answers '("t1")
      (rdm-task t))
    (should (equal (cadr calls)
                   (rdm-ui-test--show "task" "list" "--status=all")))))

(ert-deftest rdm-pick/search-fallback-argv ()
  "Without consult, search reads a query, runs `rdm search' and opens the hit."
  (rdm-ui-test--mocked
    (let ((rdm-search-use-consult nil))
      (cl-letf (((symbol-function 'read-string) (lambda (&rest _) "two")))
        (rdm-ui-test--answers '("r1/phase-2-two")
          (rdm-search))))
    (should (equal (reverse calls)
                   (list (rdm-ui-test--show "search" "two"
                                            "--type=roadmap,phase,task")
                         (rdm-ui-test--show "phase" "show" "phase-2-two"
                                            "--roadmap=r1"))))))

(ert-deftest rdm-pick/candidates-and-metadata ()
  "Candidates carry `rdm-item', the table's category is `rdm-item', and
annotations show status and tags (kind, status and tags for search hits)."
  (rdm-ui-test--mocked
    (let* ((tasks (rdm-pick-task-candidates))
           (md (completion-metadata "" (rdm-pick-table tasks) nil))
           (annotate (completion-metadata-get md 'annotation-function)))
      (should (eq (completion-metadata-get md 'category) 'rdm-item))
      (let ((item (get-text-property 0 'rdm-item (car tasks))))
        (should (eq (plist-get item :kind) 'task))
        (should (equal (plist-get item :slug) "t1"))
        (should (equal (plist-get item :ref) "task/t1"))
        (should (equal (plist-get item :review-ref) "task/t1")))
      (let ((note (funcall annotate (car tasks))))
        (should (string-match-p "open" note))
        (should (string-match-p "bug" note)))
      (let ((roadmap (car (rdm-pick-roadmap-candidates))))
        (should (string-match-p "1/2 done" (funcall annotate roadmap)))
        (should (string-match-p "ui" (funcall annotate roadmap))))
      (let ((phase (cadr (rdm-pick-phase-candidates "r1"))))
        (should (equal (get-text-property 0 'rdm-item phase)
                       (rdm-item-from "phase-2-two")))
        (should (equal (plist-get (rdm-item-from phase) :review-ref)
                       "phase/r1/phase-2-two"))
        (should (string-match-p "in-progress" (funcall annotate phase))))
      (let* ((hits (rdm-pick-search-candidates "x"))
             (items (mapcar #'rdm-item-from hits)))
        (should (equal (mapcar (lambda (i) (plist-get i :kind)) items)
                       '(roadmap phase task)))
        (should (equal (plist-get (nth 1 items) :roadmap) "r1"))
        (should (equal (plist-get (nth 1 items) :stem) "phase-2-two"))
        (let ((note (funcall annotate (nth 1 hits))))
          (should (string-match-p "phase" note))
          (should (string-match-p "in-progress" note)))
        (let ((note (funcall annotate (nth 2 hits))))
          (should (string-match-p "task" note))
          (should (string-match-p "open" note))
          (should (string-match-p "bug" note)))))))

(ert-deftest rdm-pick/next-opens-phase ()
  "`rdm-next' runs `next --roadmap' and opens the phase it names."
  (rdm-ui-test--mocked
    (let ((buf (rdm-next "r1")))
      (should (equal (cadr calls) (rdm-ui-test--show "next" "--roadmap=r1")))
      (should (equal (car calls)
                     (rdm-ui-test--show "phase" "show" "phase-2-two"
                                        "--roadmap=r1")))
      (should (equal (plist-get (buffer-local-value 'rdm-view--item buf) :stem)
                     "phase-2-two")))))

(ert-deftest rdm-pick/search-candidates-are-refs-of-structured-hits ()
  "Search asks rdm for roadmap, phase and task hits only, offers each by
its `ref' (unique across kinds), and builds the item from the hit's
structured fields; the property-less text recovers it."
  (rdm-ui-test--mocked
    (let ((hits (rdm-pick-search-candidates "x")))
      (should (equal (car calls)
                     (rdm-ui-test--show "search" "x" "--type=roadmap,phase,task")))
      (should (equal (mapcar #'substring-no-properties hits)
                     '("r1" "r1/phase-2-two" "task/t1")))
      (let ((task (rdm-item-from "task/t1")))
        (should (eq (plist-get task :kind) 'task))
        (should (equal (plist-get task :slug) "t1"))
        (should (equal (plist-get task :status) "open"))
        (should (equal (plist-get task :review-ref) "task/t1")))
      (let ((phase (rdm-item-from "r1/phase-2-two")))
        (should (equal (list (plist-get phase :roadmap) (plist-get phase :stem)
                             (plist-get phase :ref) (plist-get phase :review-ref))
                       '("r1" "phase-2-two" "r1/phase-2-two"
                         "phase/r1/phase-2-two")))))))

(ert-deftest rdm-view/identifier-falls-back ()
  "An item's identifier is the `:ref' rdm reported; an item with none (a
plan or change review target) falls back to its `:identifier'."
  (let ((kill-ring nil))
    (should (equal (rdm-item-identifier
                    '(:kind phase :roadmap "r" :stem "s" :ref "r/phase-1-s"))
                   "r/phase-1-s"))
    (should (equal (rdm-item-identifier
                    '(:kind review :identifier "2026-09-25-0000-abcd"))
                   "2026-09-25-0000-abcd"))
    (should-error (rdm-item-identifier '(:kind review)) :type 'user-error)
    (should-error (rdm-item-identifier '(:kind task :slug "t1"))
                  :type 'user-error)))

(ert-deftest rdm-view/task-identifier-is-its-ref ()
  "A bare task item opened by slug takes rdm's `ref' from `show': its
buffer name, Task field and copied identifier are `task/<slug>'."
  (rdm-ui-test--mocked
    (with-current-buffer (rdm-open-item '(:kind task :slug "t1"))
      (should (equal (buffer-name) "*rdm task task/t1*"))
      (should (equal (plist-get rdm-view--item :ref) "task/t1"))
      (should (string-match-p "^Task: task/t1$" (buffer-string)))
      (should (equal (rdm-view-copy-identifier) "task/t1"))
      (should (equal (current-kill 0) "task/t1")))))

(ert-deftest rdm-pick/next-blocked ()
  "A roadmap blocked on dependencies is a `user-error' naming every unmet
dependency, non-string entries included."
  (rdm-ui-test--mocked
    (let ((err (should-error (rdm-next "blocked") :type 'user-error)))
      (should (equal (cadr err)
                     "rdm: blocked is blocked on r0, (:roadmap \"r9\")")))
    (should (equal calls (list (rdm-ui-test--show "next" "--roadmap=blocked"))))))

(ert-deftest rdm-pick/next-nothing ()
  "A roadmap with nothing actionable returns nil and opens nothing."
  (rdm-ui-test--mocked
    (should-not (rdm-next "empty"))
    (should (equal calls (list (rdm-ui-test--show "next" "--roadmap=empty"))))))

(ert-deftest rdm-pick/next-worktree-roadmap ()
  "Interactively, a roadmap or phase worktree names the roadmap; a task
worktree falls back to the roadmap picker."
  (dolist (wt '((:kind "roadmap" :roadmap "r1" :ref "r1")
                (:kind "phase" :roadmap "r1" :stem "phase-2-two"
                       :ref "r1/phase-2-two")))
    (rdm-ui-test--mocked
      (cl-letf (((symbol-function 'rdm-worktree-item)
                 (lambda () wt))
                ((symbol-function 'completing-read)
                 (lambda (&rest _) (error "Unexpected prompt"))))
        (call-interactively #'rdm-next))
      (should (equal (cadr calls) (rdm-ui-test--show "next" "--roadmap=r1")))
      (should-not (member (rdm-ui-test--show "roadmap" "list") calls))))
  (rdm-ui-test--mocked
    (cl-letf (((symbol-function 'rdm-worktree-item)
               (lambda () (list :kind "task" :slug "t1" :ref "task/t1"))))
      (rdm-ui-test--answers '("r1")
        (call-interactively #'rdm-next)))
    (should (equal (reverse (cdr calls))
                   (list (rdm-ui-test--show "roadmap" "list")
                         (rdm-ui-test--show "next" "--roadmap=r1"))))))

;;;; Detail buffer

(ert-deftest rdm-view/read-only-body ()
  "The detail buffer is a read-only `special-mode' buffer showing the body."
  (rdm-ui-test--mocked
    (with-current-buffer (rdm-open-item '(:kind task :slug "t1"))
      (should (derived-mode-p 'special-mode))
      (should (eq major-mode 'rdm-view-mode))
      (should buffer-read-only)
      (should (string-match-p "Task body\\." (buffer-string)))
      (should (string-match-p "Status: open" (buffer-string)))
      (should (string-match-p "Tags: bug" (buffer-string)))
      (should-error (insert "x") :type 'buffer-read-only))))

(ert-deftest rdm-view/refresh-reissues-argv ()
  "`g' re-issues the same `show' argv; the opened buffer's item carries
the `ref' rdm's `show' reported."
  (rdm-ui-test--mocked
    (with-current-buffer (rdm-open-item '(:kind phase :roadmap "r1"
                                          :stem "phase-2-two"))
      (should (equal (buffer-name) "*rdm phase r1/phase-2-two*"))
      (should (equal (plist-get rdm-view--item :ref) "r1/phase-2-two"))
      (let ((first (car calls)))
        (setq calls nil)
        (call-interactively (key-binding (kbd "g")))
        (should (equal calls (list first)))))))

(ert-deftest rdm-view/roadmap-rows-open-phase ()
  "A roadmap view lists its phases as rows; RET opens the row's phase."
  (rdm-ui-test--mocked
    (with-current-buffer (rdm-open-item '(:kind roadmap :roadmap "r1"))
      (should (string-match-p "Roadmap body\\." (buffer-string)))
      (goto-char (point-min))
      (search-forward "phase-2-two")
      (should (equal (get-text-property (point) 'rdm-item)
                     '(:kind phase :roadmap "r1" :stem "phase-2-two"
                             :title "Two" :status "in-progress"
                             :ref "r1/phase-2-two"
                             :review-ref "phase/r1/phase-2-two")))
      (call-interactively (key-binding (kbd "RET")))
      (should (equal (car calls)
                     (rdm-ui-test--show "phase" "show" "phase-2-two"
                                        "--roadmap=r1")))
      (should (eq (plist-get (buffer-local-value 'rdm-view--item
                                                 (window-buffer))
                             :kind)
                  'phase)))))

(ert-deftest rdm-view/inherits-source-context ()
  "A detail buffer inherits its source buffer's buffer-local context
\(`rdm-project' and a mise.el-style `process-environment'), and `g' and
RET on a phase row resolve that same context rather than the global one."
  (let ((calls nil)
        (rdm-item--lookup (make-hash-table :test #'equal))
        (rdm-project "global-proj")
        (process-environment (cons "RDM_ROOT=/global-root"
                                   (rdm-test-clean-environment)))
        (source (generate-new-buffer " *rdm-source*")))
    (cl-letf (((symbol-function 'rdm-context)
               (lambda () (list :root (getenv "RDM_ROOT") :project rdm-project
                                :rdm-bin "rdm")))
              ((symbol-function 'rdm-run-sync)
               (lambda (command &rest keys)
                 (if (equal (seq-take (cdr command) 2) '("review" "list"))
                     []
                   (push (list (cdr command) (plist-get keys :env)) calls)
                   (rdm-ui-test--reply (cdr command))))))
      (unwind-protect
          (let* ((env (cons "RDM_ROOT=/src-root" (rdm-test-clean-environment)))
                 (view (with-current-buffer source
                         (setq-local rdm-project "src-proj")
                         (setq-local process-environment env)
                         (rdm-open-item '(:kind roadmap :roadmap "r1"))))
                 (expected (lambda (&rest words)
                             (list (append words '("--project" "src-proj"
                                                   "--format" "json"))
                                   '("RDM_ROOT=/src-root")))))
            (should (equal (car calls) (funcall expected "roadmap" "show" "r1")))
            (should (local-variable-p 'rdm-project view))
            (should (equal (buffer-local-value 'rdm-project view) "src-proj"))
            (should (local-variable-p 'process-environment view))
            (should (equal (buffer-local-value 'process-environment view) env))
            (with-current-buffer view
              (setq calls nil)
              (revert-buffer)
              (should (equal calls
                             (list (funcall expected "roadmap" "show" "r1"))))
              (goto-char (point-min))
              (search-forward "phase-2-two")
              (call-interactively (key-binding (kbd "RET"))))
            (should (equal (car calls)
                           (funcall expected "phase" "show" "phase-2-two"
                                    "--roadmap=r1")))
            (let ((phase (window-buffer)))
              (should (eq (plist-get (buffer-local-value 'rdm-view--item phase)
                                     :kind)
                          'phase))
              (should (equal (buffer-local-value 'rdm-project phase)
                             "src-proj"))
              (should (equal (buffer-local-value 'process-environment phase)
                             env))))
        (kill-buffer source)
        (rdm-ui-test--kill-views)))))

(ert-deftest rdm-view/at-threads-revision ()
  "`rdm-view-at' threads `--at' and shows the revision in the header."
  (rdm-ui-test--mocked
    (with-current-buffer (rdm-open-item '(:kind task :slug "t1"))
      (with-current-buffer (rdm-view-at "HEAD~1")
        (should (equal (car calls)
                       (rdm-ui-test--show "task" "show" "t1" "--at=HEAD~1")))
        (should (string-match-p "Revision: HEAD~1" (buffer-string)))
        (should (string-match-p "Old task body\\." (buffer-string)))
        (setq calls nil)
        (revert-buffer)
        (should (equal calls (list (rdm-ui-test--show "task" "show" "t1"
                                                      "--at=HEAD~1"))))))))

(ert-deftest rdm-view/copy-identifier-and-done-line ()
  "Copying the identifier needs no rdm call; the Done: line uses `hook done-line'."
  (rdm-ui-test--mocked
    (with-current-buffer (rdm-open-item '(:kind roadmap :roadmap "r1"))
      (should (equal (rdm-view-copy-identifier) "r1"))
      (should-error (rdm-view-copy-done-line) :type 'user-error)
      (goto-char (point-min))
      (search-forward "phase-2-two")
      (should (equal (rdm-view-copy-identifier) "r1/phase-2-two"))
      (setq calls nil)
      (should (equal (rdm-view-copy-done-line) "Done: r1/phase-2-two"))
      (should (equal calls '(("hook" "done-line" "--roadmap=r1"
                              "--phase=phase-2-two" "--format" "json"))))
      (should (equal (current-kill 0) "Done: r1/phase-2-two")))))

(ert-deftest rdm-view/vanilla-keys ()
  "Without evil, `g', `q' and `RET' resolve to refresh, quit and open."
  (with-temp-buffer
    (rdm-view-mode)
    (should (eq (key-binding (kbd "g")) #'revert-buffer))
    (should (eq (key-binding (kbd "q")) #'quit-window))
    (should (eq (key-binding (kbd "RET")) #'rdm-view-open-at-point))))

(ert-deftest rdm-view/markdown-faces ()
  "Under markdown-mode, the body keeps its text and carries markdown's
faces: a heading and a **bold** span are fontified on exactly their
characters."
  :tags '(markdown-mode)
  (rdm-test-with-soft-dep 'markdown-mode
    (rdm-ui-test--mocked
      (cl-letf* ((reply (symbol-function 'rdm-ui-test--reply))
                 ((symbol-function 'rdm-ui-test--reply)
                  (lambda (argv)
                    (let ((data (funcall reply argv)))
                      (plist-put data :body "# Heading\n\nsome **bold** text\n")))))
        (with-current-buffer (rdm-open-item '(:kind task :slug "t1"))
          (should (string-match-p "# Heading\n\nsome \\*\\*bold\\*\\* text\n"
                                  (buffer-string)))
          (goto-char (point-min))
          (search-forward "# Heading")
          (let ((h (- (point) (length "Heading"))))
            (should (memq 'markdown-header-face-1
                          (ensure-list (get-text-property h 'font-lock-face))))
            (should (memq 'markdown-header-face-1
                          (ensure-list (get-text-property (1- (point))
                                                          'font-lock-face)))))
          (search-forward "some **bold**")
          (let ((b (- (point) (length "bold**"))))
            (should (memq 'markdown-bold-face
                          (ensure-list (get-text-property b 'font-lock-face))))
            (should (memq 'markdown-bold-face
                          (ensure-list (get-text-property (+ b 3)
                                                          'font-lock-face))))
            (should-not (memq 'markdown-bold-face
                              (ensure-list (get-text-property
                                            (- b 4) 'font-lock-face))))))))))

;;;; Embark

(ert-deftest rdm-embark/map-binds-actions ()
  "The embark map binds open, open other window, copy identifier, copy Done:."
  (should (eq (keymap-lookup rdm-item-embark-map "RET") #'rdm-item-open))
  (should (eq (keymap-lookup rdm-item-embark-map "o") #'rdm-item-open-other-window))
  (should (eq (keymap-lookup rdm-item-embark-map "w") #'rdm-item-copy-identifier))
  (should (eq (keymap-lookup rdm-item-embark-map "d") #'rdm-item-copy-done-line)))

(ert-deftest rdm-embark/action-recovers-item ()
  "An action handed a property-less candidate through the minibuffer
recovers its item from the picker's lookup."
  (rdm-ui-test--mocked
    (rdm-pick-task-candidates)
    (cl-letf (((symbol-function 'read-string) (lambda (&rest _) "t1")))
      (should (equal (call-interactively #'rdm-item-copy-identifier) "task/t1"))
      (call-interactively #'rdm-item-open))
    (should (equal (car calls) (rdm-ui-test--show "task" "show" "t1")))))

(ert-deftest rdm-embark/keymap-alist ()
  "Under embark, category `rdm-item' maps to `rdm-item-embark-map'."
  :tags '(embark)
  (rdm-test-with-soft-dep 'embark
    (should (memq 'rdm-item-embark-map
                  (alist-get 'rdm-item embark-keymap-alist)))))

;;;; Consult

(ert-deftest rdm-consult/dynamic-search ()
  "The consult collection runs `rdm search INPUT' per input and yields
propertized candidates."
  :tags '(consult)
  (rdm-test-with-soft-dep 'consult
    (rdm-ui-test--mocked
      (let* ((consult-async-input-throttle 0)
             (consult-async-input-debounce 0)
             (consult-async-min-input 1)
             (got nil)
             (sink (lambda (action)
                     (when (and (consp action) (stringp (car action)))
                       (setq got (append got action)))
                     nil))
             (async (funcall (rdm-pick-search-collection) sink)))
        (with-temp-buffer
          (funcall async 'setup)
          (dolist (input '("two" "task"))
            (setq got nil)
            (funcall async input)
            (let ((deadline (+ (float-time) 5)))
              (while (and (null got) (< (float-time) deadline))
                (accept-process-output nil 0.02)))
            (should (equal (car calls) (rdm-ui-test--show "search" input
                                                          "--type=roadmap,phase,task")))
            (should (eq (plist-get (rdm-item-from (car got)) :kind) 'roadmap))
            (should (get-text-property 0 'rdm-item (car got)))))
        (funcall async 'destroy)))))

(ert-deftest rdm-consult/context-error-up-front ()
  "An unresolved context is a `user-error' before consult reads anything."
  :tags '(consult)
  (rdm-test-with-soft-dep 'consult
    (rdm-ui-test--mocked
      (cl-letf (((symbol-function 'rdm-context)
                 (lambda () (user-error "rdm: no plan root resolved")))
                ((symbol-function 'consult--read)
                 (lambda (&rest _) (error "consult--read was called"))))
        (let ((rdm-search-use-consult t)
              (err (should-error (rdm-search) :type 'user-error)))
          (should (equal (cadr err) "rdm: no plan root resolved"))))
      (should-not calls))))

(ert-deftest rdm-consult/search-error-shown ()
  "A failing search inside the live collection yields no candidates and
shows its error with `message' instead of dropping it."
  :tags '(consult)
  (rdm-test-with-soft-dep 'consult
    (rdm-ui-test--mocked
      (let* ((consult-async-input-throttle 0)
             (consult-async-input-debounce 0)
             (consult-async-min-input 1)
             (got nil)
             (shown nil)
             (sink (lambda (action)
                     (when (and (consp action) (stringp (car action)))
                       (setq got (append got action)))
                     nil))
             (async (funcall (rdm-pick-search-collection) sink)))
        (cl-letf (((symbol-function 'rdm-run-sync)
                   (lambda (&rest _) (user-error "rdm: search exploded")))
                  ((symbol-function 'message)
                   (lambda (fmt &rest args)
                     (when fmt (push (apply #'format fmt args) shown)))))
          (with-temp-buffer
            (funcall async 'setup)
            (funcall async "two")
            (let ((deadline (+ (float-time) 5)))
              (while (and (not (member "rdm: search exploded" shown))
                          (< (float-time) deadline))
                (accept-process-output nil 0.02)))
            (funcall async 'destroy)))
        (should (member "rdm: search exploded" shown))
        (should-not got)))))

;;;; Evil

(ert-deftest rdm-evil/normal-state-bindings ()
  "Under evil, a detail buffer starts in normal state with rdm's keys."
  :tags '(evil)
  (rdm-test-with-soft-dep 'evil
    (with-temp-buffer
      (rdm-view-mode)
      (evil-local-mode 1)
      (should (eq evil-state 'normal))
      (should (eq (key-binding (kbd "gr")) #'revert-buffer))
      (should (eq (key-binding (kbd "q")) #'quit-window))
      (should (eq (key-binding (kbd "RET")) #'rdm-view-open-at-point))
      (should (eq (key-binding (kbd "y")) #'rdm-view-copy-identifier))
      (should (eq (key-binding (kbd "gd")) #'rdm-view-copy-done-line))
      (should (eq (key-binding (kbd "j")) #'evil-next-line))
      (evil-local-mode -1))))

;;;; Against the real binary

(defmacro rdm-ui-test--with-real (&rest body)
  "Run BODY in the seeded plan with RDM_ROOT/RDM_PROJECT and `rdm-bin' set."
  (declare (indent 0) (debug t))
  `(rdm-test-with-real
     (setq process-environment
           (append (list (concat "RDM_ROOT=" root)
                         (concat "RDM_PROJECT=" project))
                   process-environment))
     (setq rdm-bin bin)
     (let ((kill-ring nil))
       (unwind-protect (progn ,@body)
         (rdm-ui-test--kill-views)))))

(ert-deftest rdm-view/real-roadmap-rows ()
  "A real roadmap view lists its phases as rows; RET opens the second."
  (rdm-ui-test--with-real
    (with-current-buffer (rdm-open-item '(:kind roadmap :roadmap "sample-roadmap"))
      (let (stems)
        (goto-char (point-min))
        (while (not (eobp))
          (when-let* ((item (get-text-property (point) 'rdm-item)))
            (push (plist-get item :stem) stems))
          (forward-line 1))
        (should (equal (nreverse stems)
                       '("phase-1-seed-one" "phase-2-seed-two"
                         "phase-3-seed-three"))))
      (goto-char (point-min))
      (search-forward "phase-2-seed-two")
      (with-current-buffer (rdm-view-open-at-point)
        (should (equal (plist-get rdm-view--item :stem) "phase-2-seed-two"))
        (should (string-match-p "Second phase: in progress\\." (buffer-string)))
        (should (string-match-p "Status: in-progress" (buffer-string)))))))

(ert-deftest rdm-view/real-at ()
  "A real `--at HEAD' view shows the revision and the seeded body."
  (rdm-ui-test--with-real
    (with-current-buffer (rdm-open-item '(:kind roadmap :roadmap "sample-roadmap"))
      (with-current-buffer (rdm-view-at "HEAD")
        (should (string-match-p "Revision: HEAD" (buffer-string)))
        (should (string-match-p "A sample roadmap for exercising"
                                (buffer-string)))))))

(ert-deftest rdm-pick/real-search ()
  "Real search hits map to items: the roadmap by kind, a phase from its
structured fields with its status, a task with its tag; each candidate
is the hit's ref."
  (rdm-ui-test--with-real
    (let* ((cands (rdm-pick-search-candidates "Sample"))
           (roadmap (cl-find-if (lambda (c) (eq (plist-get (rdm-item-from c) :kind)
                                                'roadmap))
                                cands)))
      (should roadmap)
      (should (equal (substring-no-properties roadmap) "sample-roadmap"))
      (should (equal (plist-get (rdm-item-from roadmap) :roadmap) "sample-roadmap"))
      (should (equal (plist-get (rdm-item-from roadmap) :review-ref)
                     "roadmap/sample-roadmap"))
      (should (string-match-p "roadmap" (rdm-pick-annotate roadmap))))
    (let* ((cands (rdm-pick-search-candidates "seed"))
           (phase (cl-find-if (lambda (c) (equal (substring-no-properties c)
                                                 "sample-roadmap/phase-2-seed-two"))
                              cands)))
      (should phase)
      (should (cl-every (lambda (c) (memq (plist-get (rdm-item-from c) :kind)
                                          '(roadmap phase task)))
                        cands))
      (let ((item (rdm-item-from phase)))
        (should (equal (plist-get item :roadmap) "sample-roadmap"))
        (should (equal (plist-get item :stem) "phase-2-seed-two"))
        (should (equal (plist-get item :status) "in-progress")))
      (should (string-match-p "phase in-progress" (rdm-pick-annotate phase))))
    (let* ((cands (rdm-pick-search-candidates "fixture"))
           (task (cl-find-if (lambda (c) (equal (plist-get (rdm-item-from c) :slug)
                                                "fixture-task-done"))
                             cands)))
      (should task)
      (should (eq (plist-get (rdm-item-from task) :kind) 'task))
      (let ((note (rdm-pick-annotate task)))
        (should (string-match-p "task" note))
        (should (string-match-p "bug" note))))))

(ert-deftest rdm-pick/real-next ()
  "Real `rdm-next' on the seeded roadmap opens its in-progress phase."
  (rdm-ui-test--with-real
    (with-current-buffer (rdm-next "sample-roadmap")
      (should (equal rdm-view--item
                     '(:kind phase :roadmap "sample-roadmap"
                             :stem "phase-2-seed-two"
                             :ref "sample-roadmap/phase-2-seed-two"
                             :review-ref "phase/sample-roadmap/phase-2-seed-two")))
      (should (string-match-p "Second phase: in progress\\." (buffer-string))))))

(ert-deftest rdm-view/real-done-line ()
  "The real `hook done-line' supplies task and phase Done: lines."
  (rdm-ui-test--with-real
    (should (equal (rdm-copy-done-line '(:kind task :slug "fixture-task-open"))
                   "Done: task/fixture-task-open"))
    (should (equal (rdm-copy-done-line '(:kind phase :roadmap "sample-roadmap"
                                               :stem "phase-2-seed-two"))
                   "Done: sample-roadmap/phase-2-seed-two"))))

(provide 'rdm-ui-test)
;;; rdm-ui-test.el ends here
