;;; rdm-compose-test.el --- ERT tests for the compose buffer and body writes  -*- lexical-binding: t; -*-

;;; Commentary:

;; The compose buffer's contract, the create and edit-body flows (against
;; the `rdm' stub of rdm-transient-test.el, which also saves a create's
;; stdin), their `rdm-dispatch' suffixes, and a `real-' round trip through
;; the binary under test.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'subr-x)
(require 'transient)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-compose)
(require 'rdm-transient)
(require 'rdm-evil)
(require 'rdm-test-helpers)
(require 'rdm-transient-stub)

(defvar evil-state)
(declare-function evil-local-mode "ext:evil-core" (&optional arg))
(declare-function evil-normal-state "ext:evil-states" (&optional arg))
(declare-function rdm-pick-read-roadmap "rdm-pick" (&optional prompt))

(defun rdm-compose-test--kill-buffers ()
  "Kill every compose buffer."
  (dolist (b (buffer-list))
    (when (eq (buffer-local-value 'major-mode b) 'rdm-compose-mode)
      (kill-buffer b))))

(defmacro rdm-compose-test--cleanly (&rest body)
  "Run BODY, then kill every compose buffer it left."
  (declare (indent 0) (debug t))
  `(unwind-protect (progn ,@body)
     (rdm-compose-test--kill-buffers)))

(defmacro rdm-compose-test--answers (answers &rest body)
  "Run BODY with `read-string' returning ANSWERS in order.
A prompt beyond ANSWERS fails the test."
  (declare (indent 1) (debug t))
  `(let ((queue (copy-sequence ,answers)))
     (cl-letf (((symbol-function 'read-string)
                (lambda (&rest _)
                  (if queue (pop queue) (error "Unexpected prompt")))))
       ,@body)))

(defmacro rdm-compose-test--created (&rest body)
  "Run BODY; return the items `rdm-create-item' returned in it, oldest first."
  (declare (indent 0) (debug t))
  `(let* ((created nil)
          (record (lambda (item) (setq created (append created (list item))) item)))
     (advice-add 'rdm-create-item :filter-return record)
     (unwind-protect (progn ,@body)
       (advice-remove 'rdm-create-item record))
     created))

(defun rdm-compose-test--refs (item)
  "ITEM's kind, `:ref' and `:review-ref', as a list."
  (list (plist-get item :kind) (plist-get item :ref) (plist-get item :review-ref)))

(defun rdm-compose-test--submit (buf text)
  "Replace BUF's text with TEXT and submit it."
  (with-current-buffer buf
    (erase-buffer)
    (insert text)
    (rdm-compose-submit)))

;;;; The compose buffer

(ert-deftest rdm-compose/submit-passes-text ()
  "Submit hands the buffer's text, unchanged, to the caller's function,
then kills the buffer."
  (rdm-compose-test--cleanly
    (let* ((got nil)
           (buf (rdm-compose "- first\n" (lambda (text) (setq got text))
                             "A test")))
      (with-current-buffer buf
        (should (derived-mode-p 'rdm-compose-mode))
        (should (equal (buffer-string) "- first\n"))
        (goto-char (point-max))
        (insert "  second line\n")
        (rdm-compose-submit))
      (should (equal got "- first\n  second line\n"))
      (should-not (buffer-live-p buf)))))

(ert-deftest rdm-compose/failed-submit-keeps-buffer ()
  "A submit function that signals leaves the buffer and its text alive."
  (rdm-compose-test--cleanly
    (let ((buf (rdm-compose "text" (lambda (_) (user-error "Refused")) "A test")))
      (with-current-buffer buf
        (should-error (rdm-compose-submit) :type 'user-error))
      (should (buffer-live-p buf))
      (should (equal (with-current-buffer buf (buffer-string)) "text")))))

(ert-deftest rdm-compose/abort-never-submits ()
  "Abort kills the buffer without calling the submit function."
  (rdm-compose-test--cleanly
    (let* ((called nil)
           (buf (rdm-compose "text" (lambda (_) (setq called t)) "A test")))
      (with-current-buffer buf (rdm-compose-abort))
      (should-not called)
      (should-not (buffer-live-p buf)))))

(ert-deftest rdm-compose/second-compose-keeps-first ()
  "A second compose opens its own buffer; the first keeps its text."
  (rdm-compose-test--cleanly
    (let ((a (rdm-compose "one" #'ignore "A"))
          (b (rdm-compose "two" #'ignore "B")))
      (should-not (eq a b))
      (should (equal (with-current-buffer a (buffer-string)) "one")))))

(ert-deftest rdm-compose/vanilla-keys ()
  "`C-c C-c' submits and `C-c C-k' aborts, without evil."
  (rdm-compose-test--cleanly
    (with-current-buffer (rdm-compose "" #'ignore "A test")
      (should (eq (key-binding (kbd "C-c C-c")) #'rdm-compose-submit))
      (should (eq (key-binding (kbd "C-c C-k")) #'rdm-compose-abort)))))

(ert-deftest rdm-evil/compose-insert-state-zz-zq ()
  "Under evil a compose buffer starts in insert state; in normal state
`ZZ' submits and `ZQ' aborts."
  :tags '(evil)
  (rdm-test-with-soft-dep 'evil
    (rdm-compose-test--cleanly
      (with-current-buffer (rdm-compose "" #'ignore "A test")
        (evil-local-mode 1)
        (should (eq evil-state 'insert))
        (evil-normal-state)
        (should (eq (key-binding (kbd "ZZ")) #'rdm-compose-submit))
        (should (eq (key-binding (kbd "ZQ")) #'rdm-compose-abort))
        (evil-local-mode -1)))))

;;;; Create

(ert-deftest rdm-compose/create-pipes-body-on-stdin ()
  "Roadmap and task creates send the composed body on stdin with
--no-edit and no --body; tags are passed as typed and empty tags send
no --tags."
  (rdm-transient-test--with-stub nil
    (rdm-compose-test--cleanly
      (let ((created
             (rdm-compose-test--created
               (rdm-compose-test--submit
                (rdm-create-roadmap "r9" "Road nine" " a , b ,")
                "Roadmap body\n- item\n")
               (should (equal (rdm-transient-test--stdin log) "Roadmap body\n- item\n"))
               (rdm-compose-test--submit
                (rdm-compose-test--answers '("t9" "Task nine" "")
                  (call-interactively #'rdm-create-task))
                "--task body"))))
        ;; The created items are what rdm's create JSON reports.
        (should (equal (mapcar #'rdm-compose-test--refs created)
                       '((roadmap "r9" "roadmap/r9") (task "task/t9" "task/t9")))))
      (should (equal (rdm-transient-test--stdin log) "--task body"))
      (should (equal (rdm-transient-test--argvs log)
                     '(("roadmap" "create" "r9" "--title=Road nine" "--tags= a , b ,"
                        "--project" "p" "--no-edit" "--format" "json")
                       ("task" "create" "t9" "--title=Task nine"
                        "--project" "p" "--no-edit" "--format" "json")))))))

(ert-deftest rdm-compose/create-empty-body ()
  "An empty compose buffer creates the item with an empty stdin."
  (rdm-transient-test--with-stub nil
    (rdm-compose-test--cleanly
      (rdm-compose-test--submit (rdm-create-task "t8" "T" nil) "")
      (should (equal (rdm-transient-test--stdin log) ""))
      (should (equal (rdm-transient-test--argvs log)
                     '(("task" "create" "t8" "--title=T" "--project" "p"
                        "--no-edit" "--format" "json")))))))

(ert-deftest rdm-compose/create-in-source-context ()
  "The create runs in the context of the buffer it was started from,
even once that buffer is gone."
  (rdm-transient-test--with-stub nil
    (rdm-compose-test--cleanly
      (let ((buf (let ((process-environment
                        (seq-remove (lambda (e) (string-prefix-p "RDM_BIN=" e))
                                    process-environment)))
                   (with-temp-buffer
                     (setq-local rdm-bin stub)
                     (rdm-create-task "t7" "T" nil)))))
        (let ((process-environment
               (seq-remove (lambda (e) (string-prefix-p "RDM_BIN=" e))
                           process-environment)))
          (rdm-compose-test--submit buf "body")))
      (should (member '("task" "create" "t7" "--title=T" "--project" "p"
                        "--no-edit" "--format" "json")
                      (rdm-transient-test--argvs log))))))

(ert-deftest rdm-compose/phase-create-roadmap-from-context ()
  "Phase create sends the bare slug and --roadmap from a roadmap item, a
phase item's parent, and the picker fallback; the number is optional and
reaches rdm as typed.  The created item is the phase rdm reports."
  (rdm-transient-test--with-stub nil
    (rdm-compose-test--cleanly
      (let ((picked nil))
        (cl-letf (((symbol-function 'rdm-pick-read-roadmap)
                   (lambda (&rest _) (setq picked t) "picked")))
          (let ((created (rdm-compose-test--created
                           (rdm-compose-test--submit
                            (rdm-compose-test--answers '("alpha" "Alpha" "" "")
                              (rdm-create-phase '(:kind roadmap :roadmap "r1")))
                            "a"))))
            (should (equal (car created)
                           '(:kind phase :roadmap "r1" :stem "phase-9-alpha"
                                   :ref "r1/phase-9-alpha"
                                   :review-ref "phase/r1/phase-9-alpha"))))
          (should (equal (rdm-transient-test--stdin log) "a"))
          (rdm-compose-test--submit
           (rdm-compose-test--answers '("beta" "Beta" "3" "x")
             (rdm-create-phase rdm-transient-test--phase))
           "b")
          (should-not picked)
          (rdm-compose-test--submit
           (rdm-compose-test--answers '("gamma" "Gamma" "" "")
             (rdm-create-phase rdm-transient-test--task))
           "c")
          (should picked)
          (rdm-compose-test--submit
           (rdm-compose-test--answers '("d" "D" "two" "")
             (rdm-create-phase '(:kind roadmap :roadmap "r1")))
           "d"))))
    (should (equal (rdm-transient-test--argvs log)
                   '(("phase" "create" "alpha" "--title=Alpha" "--roadmap=r1"
                      "--project" "p" "--no-edit" "--format" "json")
                     ("phase" "create" "beta" "--title=Beta" "--tags=x"
                      "--roadmap=r1" "--number=3" "--project" "p" "--no-edit"
                      "--format" "json")
                     ("phase" "create" "gamma" "--title=Gamma"
                      "--roadmap=picked" "--project" "p" "--no-edit"
                      "--format" "json")
                     ("phase" "create" "d" "--title=D" "--roadmap=r1"
                      "--number=two" "--project" "p" "--no-edit"
                      "--format" "json"))))))

(ert-deftest rdm-compose/phase-create-from-dispatch ()
  "The transient's phase suffix takes the roadmap from the transient's item."
  (rdm-transient-test--with-stub nil
    (rdm-compose-test--cleanly
      (cl-letf (((symbol-function 'transient-scope)
                 (lambda (&rest _) '(:kind roadmap :roadmap "r5"))))
        (rdm-compose-test--submit
         (rdm-compose-test--answers '("delta" "Delta" "" "")
           (rdm-dispatch-create-phase))
         "d")))
    (should (equal (rdm-transient-test--argvs log)
                   '(("phase" "create" "delta" "--title=Delta" "--roadmap=r5"
                      "--project" "p" "--no-edit" "--format" "json"))))))

;;;; Edit body

(ert-deftest rdm-compose/edit-body-argv ()
  "Edit body starts from the current body and writes --body; a blank
buffer writes --clear-body and no --body."
  (rdm-transient-test--with-stub nil
    (rdm-compose-test--cleanly
      (let ((buf (rdm-edit-body rdm-transient-test--task)))
        (should (equal (with-current-buffer buf (buffer-string)) "B"))
        (rdm-compose-test--submit buf "- new body"))
      (rdm-compose-test--submit (rdm-edit-body rdm-transient-test--phase) " \n\t"))
    (should (equal (seq-remove (lambda (argv) (equal (cadr argv) "show"))
                               (rdm-transient-test--argvs log))
                   '(("task" "update" "t1" "--body=- new body" "--project" "p"
                      "--no-edit")
                     ("phase" "update" "phase-2-two" "--roadmap=r1"
                      "--clear-body" "--project" "p" "--no-edit"))))))

;;;; The transient

(defun rdm-compose-test--dispatch-keys (item)
  "The suffix keys `rdm-dispatch' offers for ITEM."
  (unwind-protect
      (progn (rdm-dispatch-item item)
             (mapcar (lambda (obj) (oref obj key)) transient--suffixes))
    (transient-quit-all)))

(ert-deftest rdm-compose/dispatch-groups ()
  "Create is offered with and without an item; Edit body only with one.
The fixed keys are never generated for a status."
  (rdm-transient-test--with-stub nil
    (let ((none (rdm-compose-test--dispatch-keys nil))
          (task (rdm-compose-test--dispatch-keys rdm-transient-test--task)))
      (dolist (key '("+r" "+p" "+t"))
        (should (member key none))
        (should (member key task)))
      (should-not (member "B" none))
      (should (member "B" task))))
  (let ((keys (rdm-dispatch--status-keys
               '("Body" "Tags" "Status" "Commit" "Discard" "+"))))
    (dolist (key rdm-dispatch--fixed-keys)
      (should-not (member key keys)))))

;;;; Against the real binary

(ert-deftest rdm-compose/real-create-edit-clear ()
  "A task created through the create flow has the composed body; edit
body replaces it and emptying the buffer clears it.  A roadmap, and a
phase in it (the bare slug numbered by rdm), get their composed bodies."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project))
                  process-environment))
    (setq rdm-bin bin)
    (rdm-compose-test--cleanly
      (let ((slug "emacs-compose-real-task")
            (body (lambda (slug)
                    (string-trim-right
                     (or (plist-get (rdm-call-sync 'task-show slug) :body) "")
                     "\n"))))
        (should (equal (mapcar #'rdm-compose-test--refs
                               (rdm-compose-test--created
                                 (rdm-compose-test--submit
                                  (rdm-compose-test--answers
                                      (list slug "Composed in Emacs" "emacs")
                                    (call-interactively #'rdm-create-task))
                                  "Composed body\n\n- a list item\n")))
                       (list (list 'task (concat "task/" slug)
                                   (concat "task/" slug)))))
        (let ((shown (rdm-call-sync 'task-show slug)))
          (should (member "emacs" (append (plist-get shown :tags) nil))))
        (should (equal (funcall body slug) "Composed body\n\n- a list item"))
        (let* ((item (list :kind 'task :slug slug :ref (concat "task/" slug)))
               (buf (rdm-edit-body item)))
          (should (equal (string-trim-right
                          (with-current-buffer buf (buffer-string)) "\n")
                         "Composed body\n\n- a list item"))
          (rdm-compose-test--submit buf "-- edited body")
          (should (equal (funcall body slug) "-- edited body"))
          (rdm-compose-test--submit (rdm-edit-body item) "")
          (should (equal (funcall body slug) "")))
        ;; A roadmap of its own, so the seeded roadmap's phases stay as
        ;; the other real tests expect them.
        (let ((roadmap "emacs-compose-real-roadmap"))
          (should (equal (mapcar #'rdm-compose-test--refs
                                 (rdm-compose-test--created
                                   (rdm-compose-test--submit
                                    (rdm-create-roadmap roadmap "Roadmap from Emacs" nil)
                                    "Roadmap body\n")))
                         (list (list 'roadmap roadmap (concat "roadmap/" roadmap)))))
          (should (equal (string-trim-right
                          (plist-get (rdm-call-sync 'roadmap-show roadmap) :body)
                          "\n")
                         "Roadmap body"))
          (let ((created (car (rdm-compose-test--created
                                (rdm-compose-test--submit
                                 (rdm-compose-test--answers
                                     '("from-emacs" "Phase from Emacs" "" "")
                                   (rdm-create-phase (list :kind 'roadmap :roadmap roadmap)))
                                 "Phase body\n")))))
            ;; The stem is the one rdm numbered, read from its create JSON.
            (should (equal created
                           (list :kind 'phase :roadmap roadmap
                                 :stem "phase-1-from-emacs"
                                 :ref (concat roadmap "/phase-1-from-emacs")
                                 :review-ref (concat "phase/" roadmap
                                                     "/phase-1-from-emacs")))))
          (let ((shown (rdm-call-sync 'phase-show "phase-1-from-emacs"
                                      :roadmap roadmap)))
            (should (equal (string-trim-right (plist-get shown :body) "\n")
                           "Phase body"))))))))

(provide 'rdm-compose-test)
;;; rdm-compose-test.el ends here
