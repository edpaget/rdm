;;; rdm-worktree-test.el --- ERT tests for the worktree commands  -*- lexical-binding: t; -*-

;;; Commentary:

;; The `rdm worktree' argv rows, and the list, add, remove and prune
;; commands against a mocked runner with the open
;; function captured (no project is ever switched).  The `real-' test
;; lives in rdm-modeline-test.el, which drives `rdm-worktree-add' through
;; to the modeline segment.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'rdm-core)
(require 'rdm-worktree)
(require 'rdm-test-helpers)
(require 'rdm-runner-mock)

(defconst rdm-worktree-test--rows
  (vector (list :item "task/t1" :branch "task/t1" :path "/code__worktrees/task-t1"
                :dirty :false)
          (list :item "r1/phase-2-two" :branch "phase/r1/phase-2-two"
                :path "/code__worktrees/phase-r1-phase-2-two" :dirty t))
  "A canned parsed `worktree list' reply.")

(defmacro rdm-worktree-test--with (reply &rest body)
  "Run BODY against a mocked runner answering REPLY; capture opens.
Binds `opened' to the directories the open function got, newest first."
  (declare (indent 1) (debug t))
  `(let ((opened nil))
     (let ((rdm-worktree-open-function (lambda (dir) (push dir opened))))
       (rdm-mock-with-runner ,reply
         ,@body))))

(defun rdm-worktree-test--reply (argv _dir)
  "Answer ARGV as a small code repo with two worktrees would."
  (pcase argv
    (`("worktree" "list" . ,_) rdm-worktree-test--rows)
    (`("worktree" "add" ,item . ,_)
     (list :item item :branch item :path (concat "/wt/" item) :created t))
    (`("worktree" "remove" ,path . ,rest)
     (list :item "r1/phase-2-two" :branch "phase/r1/phase-2-two" :path path
           :kind "phase" :roadmap "r1" :stem "phase-2-two" :ref "r1/phase-2-two"
           :review_ref "phase/r1/phase-2-two"
           :branch_deleted (if (member "--delete-branch" rest) t :false)))
    (`("worktree" "prune" . ,rest)
     (list :results (vector (list :item "task/t1" :branch "task/t1"
                                  :path "/code__worktrees/task-t1"
                                  :action (if (member "--dry-run" rest)
                                              "would-remove"
                                            "removed")))
           :removed (if (member "--dry-run" rest) 0 1)))
    (`("task" "list" . ,_) (vector (list :slug "t1" :title "T" :status "open"
                                        :ref "task/t1" :review_ref "task/t1")))
    (`("roadmap" "list" . ,_) (vector (list :slug "r1" :title "R" :ref "r1"
                                           :review_ref "roadmap/r1")))
    (`("phase" "list" . ,_) (vector (list :stem "phase-2-two" :title "Two"
                                         :status "in-progress"
                                         :ref "r1/phase-2-two"
                                         :review_ref "phase/r1/phase-2-two")))
    (`("task" "show" ,slug . ,_)
     (list :title "T" :status "open" :body ""
           :ref (concat "task/" slug) :review_ref (concat "task/" slug)))
    (`("roadmap" "show" ,slug . ,_)
     (list :title "R" :body "" :ref slug :review_ref (concat "roadmap/" slug)
           :phases (vector (list :number 2 :stem "phase-2-two" :title "Two"
                                 :status "in-progress"
                                 :ref (concat slug "/phase-2-two")
                                 :review_ref (format "phase/%s/phase-2-two" slug)))))
    (_ (error "Unexpected argv %S" argv))))

;;;; Argv

(ert-deftest rdm-worktree/argv ()
  "The worktree rows: list takes no --project; add, remove, current and
prune are JSON reads."
  (should (equal (car (rdm-argv 'worktree-list))
                 '("worktree" "list" "--format" "json")))
  (should (equal (car (rdm-argv 'worktree-add "task/x" :project "P"))
                 '("worktree" "add" "task/x" "--project" "P" "--format" "json")))
  (should (equal (car (rdm-argv 'worktree-remove "/wt/x" :force t :project "P"))
                 '("worktree" "remove" "/wt/x" "--force" "--project" "P"
                   "--format" "json")))
  (should (equal (car (rdm-argv 'worktree-current :project "P"))
                 '("worktree" "current" "--project" "P" "--format" "json")))
  (should (equal (car (rdm-argv 'worktree-prune :dry-run t :project "P"))
                 '("worktree" "prune" "--dry-run" "--project" "P"
                   "--format" "json"))))

;;;; List

(ert-deftest rdm-worktree/list-opens-row ()
  "Picking a `worktree list' row opens its path through the open function."
  (rdm-worktree-test--with #'rdm-worktree-test--reply
    (cl-letf (((symbol-function 'completing-read)
               (lambda (_prompt table &rest _)
                 (should (equal (all-completions "" table)
                                '("task/t1" "r1/phase-2-two")))
                 "r1/phase-2-two")))
      (rdm-worktree-list))
    (should (equal opened '("/code__worktrees/phase-r1-phase-2-two")))
    (should (equal (rdm-mock-argvs calls)
                   '(("worktree" "list" "--format" "json"))))))

;;;; Add

(ert-deftest rdm-worktree/add-argv-and-open ()
  "`rdm-worktree-add' sends each item's `:ref' and opens the path rdm
returns."
  (rdm-worktree-test--with #'rdm-worktree-test--reply
    (rdm-worktree-add '(:kind task :slug "t1" :ref "task/t1"))
    (rdm-worktree-add '(:kind phase :roadmap "r1" :stem "phase-2-two"
                              :ref "r1/phase-2-two"))
    (rdm-worktree-add '(:kind roadmap :roadmap "r1" :ref "r1"))
    (should (equal (rdm-mock-argvs calls)
                   '(("worktree" "add" "task/t1" "--project" "p" "--format" "json")
                     ("worktree" "add" "r1/phase-2-two" "--project" "p"
                      "--format" "json")
                     ("worktree" "add" "r1" "--project" "p" "--format" "json"))))
    (should (equal opened '("/wt/r1" "/wt/r1/phase-2-two" "/wt/task/t1")))))

(ert-deftest rdm-worktree/add-picks-item ()
  "Without an item at point, `rdm-worktree-add' picks one."
  (rdm-worktree-test--with #'rdm-worktree-test--reply
    (let ((answers '("phase" "r1" "phase-2-two")))
      (cl-letf (((symbol-function 'completing-read)
                 (lambda (&rest _) (pop answers))))
        (call-interactively #'rdm-worktree-add)))
    (should (member '("worktree" "add" "r1/phase-2-two" "--project" "p"
                      "--format" "json")
                    (rdm-mock-argvs calls)))
    (should (equal opened '("/wt/r1/phase-2-two")))))

(ert-deftest rdm-worktree/add-picks-roadmap ()
  "A picked roadmap is a roadmap candidate, so its `:ref' is sent."
  (rdm-worktree-test--with #'rdm-worktree-test--reply
    (let ((answers '("roadmap" "r1")))
      (cl-letf (((symbol-function 'completing-read)
                 (lambda (&rest _) (pop answers))))
        (call-interactively #'rdm-worktree-add)))
    (should (member '("worktree" "add" "r1" "--project" "p" "--format" "json")
                    (rdm-mock-argvs calls)))))

(ert-deftest rdm-worktree/add-from-detail-buffer-sends-ref ()
  "From a detail buffer the shown item's `:ref' (from its `show' JSON) is
sent; on a roadmap buffer's phase row, that phase's `ref'."
  (rdm-worktree-test--with #'rdm-worktree-test--reply
    (with-current-buffer (rdm-open-item '(:kind task :slug "t1"))
      (call-interactively #'rdm-worktree-add))
    (with-current-buffer (rdm-open-item '(:kind roadmap :roadmap "r1"))
      (goto-char (point-min))
      (search-forward "phase-2-two")
      (call-interactively #'rdm-worktree-add))
    (should (equal (seq-filter (lambda (argv) (equal (car argv) "worktree"))
                               (rdm-mock-argvs calls))
                   '(("worktree" "add" "task/t1" "--project" "p" "--format" "json")
                     ("worktree" "add" "r1/phase-2-two" "--project" "p"
                      "--format" "json"))))))

(ert-deftest rdm-worktree/add-without-ref-is-a-user-error ()
  "An item rdm gave no `:ref' has no worktree: a `user-error', no rdm run."
  (rdm-worktree-test--with #'rdm-worktree-test--reply
    (should-error (rdm-worktree-add '(:kind task :slug "t1")) :type 'user-error)
    (should-error (rdm-worktree-add '(:kind plan :slug "p")) :type 'user-error)
    (should-not calls)))

;;;; Remove and prune

(ert-deftest rdm-worktree/remove-confirmed ()
  "Declining runs nothing; accepting removes the path, with
--delete-branch under a prefix and --force too under two."
  (let ((row (aref rdm-worktree-test--rows 1)))
    (rdm-worktree-test--with #'rdm-worktree-test--reply
      (let (prompts)
        (cl-letf (((symbol-function 'y-or-n-p)
                   (lambda (prompt) (push prompt prompts) nil)))
          (should-not (rdm-worktree-remove row)))
        (should (string-match-p "uncommitted" (car prompts))))
      (should-not calls)
      (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
        (let ((reply (rdm-worktree-remove row)))
          (should (equal (plist-get reply :ref) "r1/phase-2-two"))
          (should (eq (plist-get reply :branch_deleted) :false)))
        (should (eq (plist-get (rdm-worktree-remove row '(4)) :branch_deleted) t))
        (rdm-worktree-remove row '(16)))
      (should (equal (rdm-mock-argvs calls)
                     '(("worktree" "remove" "/code__worktrees/phase-r1-phase-2-two"
                        "--project" "p" "--format" "json")
                       ("worktree" "remove" "/code__worktrees/phase-r1-phase-2-two"
                        "--delete-branch" "--project" "p" "--format" "json")
                       ("worktree" "remove" "/code__worktrees/phase-r1-phase-2-two"
                        "--delete-branch" "--force" "--project" "p"
                        "--format" "json")))))))

(ert-deftest rdm-worktree/prune-dry-run-first ()
  "Prune dry-runs first; declining stops there, accepting prunes."
  (rdm-worktree-test--with #'rdm-worktree-test--reply
    (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) nil)))
      (should-not (rdm-worktree-prune)))
    (should (equal (rdm-mock-argvs calls)
                   '(("worktree" "prune" "--dry-run" "--project" "p"
                      "--format" "json"))))
    (setq calls nil)
    (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
      (should (equal (plist-get (rdm-worktree-prune '(4)) :removed) 1)))
    (should (equal (rdm-mock-argvs calls)
                   '(("worktree" "prune" "--dry-run" "--delete-branch"
                      "--project" "p" "--format" "json")
                     ("worktree" "prune" "--delete-branch" "--project" "p"
                      "--format" "json"))))))

(ert-deftest rdm-worktree/prune-nothing ()
  "With nothing done, prune asks nothing and runs only the dry run."
  (rdm-worktree-test--with
      (lambda (argv _dir)
        (pcase argv
          (`("worktree" "prune" . ,_) (list :results [] :would_remove 0))
          (_ (error "Unexpected argv %S" argv))))
    (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) (error "Asked"))))
      (should-not (rdm-worktree-prune)))
    (should (= (length calls) 1))))

(provide 'rdm-worktree-test)
;;; rdm-worktree-test.el ends here
