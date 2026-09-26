;;; rdm-worktree-test.el --- ERT tests for the worktree commands  -*- lexical-binding: t; -*-

;;; Commentary:

;; The `rdm worktree' argv rows, the item-string helpers, and the list,
;; add, remove and prune commands against a mocked runner with the open
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
    (`("worktree" "remove" . ,_) "Removed worktree\n")
    (`("worktree" "prune" . ,rest)
     (list :results (vector (list :item "task/t1" :branch "task/t1"
                                  :path "/code__worktrees/task-t1"
                                  :action (if (member "--dry-run" rest)
                                              "would-remove"
                                            "removed")))
           :removed (if (member "--dry-run" rest) 0 1)))
    (`("task" "list" . ,_) (vector (list :slug "t1" :title "T" :status "open")))
    (`("roadmap" "list" . ,_) (vector (list :slug "r1" :title "R")))
    (`("phase" "list" . ,_) (vector (list :stem "phase-2-two" :title "Two"
                                         :status "in-progress")))
    (_ (error "Unexpected argv %S" argv))))

;;;; Argv and item strings

(ert-deftest rdm-worktree/argv ()
  "The worktree rows: list takes no --project, add and prune are JSON
reads, remove is text (neither --format nor --no-edit)."
  (should (equal (car (rdm-argv 'worktree-list))
                 '("worktree" "list" "--format" "json")))
  (should (equal (car (rdm-argv 'worktree-add "task/x" :project "P"))
                 '("worktree" "add" "task/x" "--project" "P" "--format" "json")))
  (should (equal (car (rdm-argv 'worktree-remove "/wt/x" :force t :project "P"))
                 '("worktree" "remove" "/wt/x" "--force" "--project" "P")))
  (should (equal (car (rdm-argv 'worktree-prune :dry-run t :project "P"))
                 '("worktree" "prune" "--dry-run" "--project" "P"
                   "--format" "json"))))

(ert-deftest rdm-worktree/item-strings ()
  "Item plists map to `rdm worktree' item strings and back."
  (dolist (case '(((:kind task :slug "t1") . "task/t1")
                  ((:kind phase :roadmap "r1" :stem "phase-2-two") . "r1/phase-2-two")
                  ((:kind roadmap :roadmap "r1") . "r1")))
    (should (equal (rdm-worktree-id (car case)) (cdr case)))
    (should (equal (rdm-worktree-parse-id (cdr case)) (car case))))
  (should-not (rdm-worktree-parse-id nil))
  (should-not (rdm-worktree-parse-id "")))

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
  "`rdm-worktree-add' sends the item string for each kind and opens the
path rdm returns."
  (rdm-worktree-test--with #'rdm-worktree-test--reply
    (rdm-worktree-add '(:kind task :slug "t1"))
    (rdm-worktree-add '(:kind phase :roadmap "r1" :stem "phase-2-two"))
    (rdm-worktree-add '(:kind roadmap :roadmap "r1"))
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
        (rdm-worktree-remove row)
        (rdm-worktree-remove row '(4))
        (rdm-worktree-remove row '(16)))
      (should (equal (rdm-mock-argvs calls)
                     '(("worktree" "remove" "/code__worktrees/phase-r1-phase-2-two"
                        "--project" "p")
                       ("worktree" "remove" "/code__worktrees/phase-r1-phase-2-two"
                        "--delete-branch" "--project" "p")
                       ("worktree" "remove" "/code__worktrees/phase-r1-phase-2-two"
                        "--delete-branch" "--force" "--project" "p")))))))

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
