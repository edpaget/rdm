;;; rdm-review-test.el --- ERT tests for the review layer  -*- lexical-binding: t; -*-

;;; Commentary:

;; The review layer of rdm-review.el: overlays and the Reviews section
;; from canned `review list' JSON (shaped like the binary's), anchor
;; placement over a changed body and multibyte text, resolution, closing,
;; the requests picker, the revision handoff and the *Review* group of
;; `rdm-dispatch' against a mocked runner; the evil keys; and a `real-'
;; round trip through the binary under test.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'seq)
(require 'subr-x)
(require 'transient)
(require 'rdm-core)
(require 'rdm-view)
(require 'rdm-transient)
(require 'rdm-review)
(require 'rdm-evil)
(require 'rdm-test-helpers)
(require 'rdm-runner-mock)
(require 'rdm-transient-stub)

(defvar evil-state)
(declare-function evil-local-mode "ext:evil-core" (&optional arg))
(declare-function evil-visual-char "ext:evil-states")
(declare-function evil-exit-visual-state "ext:evil-states")

;;;; Fixtures

(defconst rdm-review-test--body
  "Intro — naïve café line.\nSecond line with target words here.\nThird: 日本語 text anchor.\nFourth 😀 smile.\n"
  "A body with 2-, 3- and 4-byte UTF-8 characters.")

(defvar rdm-review-test--current rdm-review-test--body
  "The body the mocked `show' returns without `--at'.")

(defvar rdm-review-test--originals nil
  "Alist of (COMMIT . BODY) the mocked `show --at' returns.")

(defvar rdm-review-test--reviews nil
  "The reviews the mocked `review list' returns.")

(defvar rdm-review-test--requests nil
  "The reviews the mocked `review requests' returns.")

(defvar rdm-review-test--update-error nil
  "When non-nil, the message the mocked `review update' fails with.")

(defvar rdm-review-test--failing nil
  "Reads the mock fails: `list' for `review list', `at' for `show --at'.")

(defvar rdm-review-test--verdicts '("approve" "request-changes" "comment")
  "The verdicts the mocked `describe review' declares.")

(defvar rdm-review-test--new-id "r-new"
  "The id the mocked `review start' gives a new draft.")

(defvar rdm-review-test--head "c-head"
  "The `created_commit' the mocked `review start' stamps.")

(defun rdm-review-test--flag-target (ref)
  "The target JSON of the reference REF (task/SLUG or phase/R/STEM)."
  (pcase (split-string ref "/")
    (`("task" ,slug) (list :kind "task" :slug slug))
    (`("phase" ,roadmap ,stem) (list :kind "phase" :roadmap roadmap :stem stem))
    (`("roadmap" ,roadmap) (list :kind "roadmap" :roadmap roadmap))))

(defun rdm-review-test--flag (argv flag)
  "The value of `--FLAG=value' in ARGV, or nil."
  (let ((prefix (concat "--" flag "=")))
    (cl-some (lambda (a) (and (string-prefix-p prefix a)
                              (substring a (length prefix))))
             argv)))

(defun rdm-review-test--reply (argv _dir)
  "A canned reply to ARGV."
  (let ((at (rdm-review-test--flag argv "at")))
    (pcase argv
      ((guard (and at (memq 'at rdm-review-test--failing)))
       (cons :mock-error (format "no such revision: %s" at)))
      ((and `("review" "list" . ,_)
            (guard (memq 'list rdm-review-test--failing)))
       (cons :mock-error "unrecognized subcommand 'list'"))
      (`(,(or "task" "phase") "show" ,_ . ,_)
       (list :title "T" :status "open"
             :body (if at
                       (cdr (assoc at rdm-review-test--originals))
                     rdm-review-test--current)))
      (`("roadmap" "show" . ,_)
       (list :title "R" :body rdm-review-test--current
             :phases (vector (list :number 1 :stem "phase-1-one"
                                   :title "One" :status "done"))))
      (`("review" "list" . ,_)
       (let ((state (rdm-review-test--flag argv "state"))
             (author (rdm-review-test--flag argv "author")))
         (vconcat (seq-filter
                   (lambda (r)
                     (and (or (not state) (equal (plist-get r :state) state))
                          (or (not author) (equal (plist-get r :author) author))))
                   rdm-review-test--reviews))))
      (`("describe" "review" . ,_)
       (list :name "review"
             :fields (vector '(:name "state" :enum_values ["draft" "submitted"])
                             (list :name "verdict"
                                   :enum_values (vconcat rdm-review-test--verdicts)))))
      (`("review" "start" . ,_)
       (let ((review (rdm-review-test--review
                      rdm-review-test--new-id
                      (rdm-review-test--flag-target (rdm-review-test--flag argv "on"))
                      nil :state "draft" :verdict nil
                      :author (rdm-review-test--flag argv "author")
                      :created_commit rdm-review-test--head
                      :created "2026-09-26T10:00:00Z")))
         (setq rdm-review-test--reviews
               (append rdm-review-test--reviews (list review)))
         review))
      (`("review" "comment" ,id . ,_) (format "Added comment 1 to review '%s'\n" id))
      (`("review" "submit" ,id . ,_)
       (when-let* ((review (seq-find (lambda (r) (equal (plist-get r :id) id))
                                     rdm-review-test--reviews)))
         (plist-put review :state "submitted")
         (plist-put review :verdict (rdm-review-test--flag argv "verdict")))
       (format "Submitted review '%s'\n" id))
      (`("review" "requests" . ,_) (vconcat rdm-review-test--requests))
      (`("review" "show" . ,_) (car rdm-review-test--reviews))
      (`("review" "update" ,id . ,_)
       (if rdm-review-test--update-error
           (cons :mock-error rdm-review-test--update-error)
         (format "Updated review '%s' → state: submitted\n" id)))
      (_ (cons :mock-error (format "unexpected %S" argv))))))

(defmacro rdm-review-test--with (reviews &rest body)
  "Run BODY with the mocked runner answering REVIEWS for `review list'.
Detail buffers are killed afterwards."
  (declare (indent 1) (debug t))
  `(let ((rdm-review-test--current rdm-review-test--body)
         (rdm-review-test--originals nil)
         (rdm-review-test--reviews ,reviews)
         (rdm-review-test--requests nil)
         (rdm-review-test--update-error nil)
         (rdm-review-test--failing nil)
         (rdm-review-test--verdicts '("approve" "request-changes" "comment"))
         (rdm-review-test--new-id "r-new")
         (rdm-review-test--head "c-head")
         (rdm-review-author "me")
         (kill-ring nil))
     (rdm-context-reset)
     (rdm-mock-with-runner #'rdm-review-test--reply
       (ignore calls contexts)
       (unwind-protect (progn ,@body)
         (dolist (b (buffer-list))
           (when (eq (buffer-local-value 'major-mode b) 'rdm-view-mode)
             (kill-buffer b)))))))

(defun rdm-review-test--comment (id quote start end &rest keys)
  "An anchored comment ID on QUOTE at bytes START..END.
KEYS: `:state' (default \"resolved\"), `:body' (\"original\"),
`:status' (\"open\"), `:doc', `:prefix', `:suffix'."
  (append
   (list :id id :status (or (plist-get keys :status) "open")
         :anchor (list :anchor_type "text-quote" :quote quote
                       :prefix (or (plist-get keys :prefix) "")
                       :suffix (or (plist-get keys :suffix) ""))
         :body (format "Comment %s." id)
         :resolution (list :state (or (plist-get keys :state) "resolved")
                           :quote quote :range_start start :range_end end
                           :body (or (plist-get keys :body) "original")))
   (when (plist-get keys :doc)
     (list :doc (list :kind "phase" :stem (plist-get keys :doc))))))

(defun rdm-review-test--review (id target comments &rest keys)
  "A review ID on TARGET (JSON) with COMMENTS; KEYS override fields."
  (append keys
          (list :id id :author "alice" :target target
                :state "submitted" :verdict "request-changes"
                :created_commit "c0" :body ""
                :comments (vconcat comments))))

(defconst rdm-review-test--task-target '(:kind "task" :slug "t1"))
(defconst rdm-review-test--task '(:kind task :slug "t1"))

(defun rdm-review-test--overlays ()
  "The review overlays of this buffer, in buffer order."
  (sort (seq-filter (lambda (ov) (overlay-get ov 'rdm-review))
                    (overlays-in (point-min) (point-max)))
        (lambda (a b) (< (overlay-start a) (overlay-start b)))))

(defun rdm-review-test--overlay-of (id)
  "The overlay of comment ID, or nil."
  (seq-find (lambda (ov)
              (equal (plist-get (plist-get (overlay-get ov 'rdm-review-entry)
                                           :comment)
                                :id)
                     id))
            (rdm-review-test--overlays)))

(defun rdm-review-test--text (ov)
  "The buffer text OV covers."
  (buffer-substring-no-properties (overlay-start ov) (overlay-end ov)))

(defun rdm-review-test--faces (ov)
  "OV's face as a list."
  (ensure-list (overlay-get ov 'face)))

(defun rdm-review-test--section ()
  "The text of the Reviews section."
  (save-excursion
    (goto-char (point-min))
    (search-forward "\nReviews\n")
    (buffer-substring-no-properties (point) (point-max))))

;;;; Placement

(ert-deftest rdm-review/byte-to-char ()
  "Byte offsets map to character offsets over multibyte text; offsets
past the end or inside a character are nil."
  (let ((s rdm-review-test--body))
    (should (= (rdm-review-byte-to-char s 0) 0))
    (should (= (rdm-review-byte-to-char s 6) 6))
    (should (= (rdm-review-byte-to-char s 9) 7))
    (should (= (rdm-review-byte-to-char s 114) (length s)))
    (should-not (rdm-review-byte-to-char s 7))
    (should-not (rdm-review-byte-to-char s 103))
    (should-not (rdm-review-byte-to-char s 115))
    (should-not (rdm-review-byte-to-char s -1))
    (should-not (rdm-review-byte-to-char s nil))))

(ert-deftest rdm-review/overlays-per-anchored-comment ()
  "One overlay per anchored comment, with its state's face; a
whole-document comment is listed, not overlaid; K describes a comment."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "target words" 46 58)
                   (list :id 2 :status "open" :body "Whole doc."
                         :resolution '(:state "unresolved"))
                   (rdm-review-test--comment 3 "日本語 text" 72 86
                                             :body "current")
                   (rdm-review-test--comment 4 "café" 17 22
                                             :status "addressed"))))
    (setq rdm-review-test--originals (list (cons "c0" rdm-review-test--body)))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (should (= (length (rdm-review-test--overlays)) 3))
      (let ((one (rdm-review-test--overlay-of 1))
            (three (rdm-review-test--overlay-of 3))
            (four (rdm-review-test--overlay-of 4)))
        (should (equal (rdm-review-test--text one) "target words"))
        (should (equal (rdm-review-test--faces one) '(rdm-review-resolved)))
        (should (equal (rdm-review-test--text three) "日本語 text"))
        (should (equal (rdm-review-test--faces four)
                       '(rdm-review-closed rdm-review-resolved)))
        (should (overlay-get one 'before-string))
        (goto-char (overlay-start one))
        (let ((text (rdm-review-comment-at-point)))
          (should (string-match-p "Review r1 #1 by alice" text))
          (should (string-match-p "open, resolved" text))
          (should (string-match-p "Comment 1\\." text))))
      (should-not (rdm-review-test--overlay-of 2))
      (let ((section (rdm-review-test--section)))
        (should (string-match-p "r1  alice  submitted request-changes  3/4 open"
                                section))
        (should (string-match-p "#2 open  whole document  Whole doc\\." section)))
      ;; A whole-document row is a comment at point too.
      (goto-char (point-min))
      (search-forward "#2 open")
      (should (string-match-p "whole document" (rdm-review-comment-at-point))))))

(ert-deftest rdm-review/anchored-unresolved-comment-listed-un-placed ()
  "An anchored comment whose resolution is `unresolved' is not a
whole-document comment: it is not overlaid and is listed un-placed with
its quote."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current")
                   (list :id 2 :status "open" :body "Lost."
                         :anchor (list :anchor_type "text-quote"
                                       :quote "vanished words"
                                       :prefix "" :suffix "")
                         :resolution '(:state "unresolved")))))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (should (rdm-review-test--overlay-of 1))
      (should-not (rdm-review-test--overlay-of 2))
      (let ((section (rdm-review-test--section)))
        (should (string-match-p
                 "#2 open  un-placed (unresolved) \"vanished words\"" section))
        (should-not (string-match-p "whole document" section))))))

(ert-deftest rdm-review/next-previous ()
  "]c / [c move between overlaid comments in buffer order."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 2 "日本語 text" 72 86 :body "current")
                   (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (goto-char (point-min))
      (rdm-review-next-comment)
      (should (looking-at-p "café"))
      (rdm-review-next-comment)
      (should (looking-at-p "日本語"))
      (should-error (rdm-review-next-comment) :type 'user-error)
      (rdm-review-previous-comment)
      (should (looking-at-p "café"))
      (should-error (rdm-review-previous-comment) :type 'user-error))))

(ert-deftest rdm-review/roadmap-doc-comment-not-placed ()
  "A roadmap review's comment on one of its phases indexes the phase body,
so it is listed, never overlaid on the roadmap body."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" '(:kind "roadmap" :roadmap "r1")
             (list (rdm-review-test--comment 1 "Intro" 0 5 :doc "phase-1-one")
                   (rdm-review-test--comment 2 "café" 17 22))))
    (setq rdm-review-test--originals (list (cons "c0" rdm-review-test--body)))
    (with-current-buffer (rdm-open-item '(:kind roadmap :roadmap "r1"))
      (should-not (rdm-review-test--overlay-of 1))
      (should (equal (rdm-review-test--text (rdm-review-test--overlay-of 2))
                     "café"))
      (should (string-match-p "#1 open  on phase phase-1-one"
                              (rdm-review-test--section))))))

(ert-deftest rdm-review/original-anchors-mapped ()
  "An `original' anchor whose offset moved lands on the quote's new
position; a drifted one whose quote survives is placed drifted, one
whose quote is gone is listed un-placed; a `current' range is used as is."
  (let ((old "Old.\nSecond line with target words here.\nA drifting phrase.\nGone text.\n")
        (new "A much longer new intro — ünïcode.\nSecond line with target words here.\nA drifting phrase!\nX 😀 current.\n"))
    (rdm-review-test--with
        (list (rdm-review-test--review
               "r1" rdm-review-test--task-target
               (list (rdm-review-test--comment 1 "target words" 22 34)
                     (rdm-review-test--comment 2 "drifting" 43 51 :state "drifted")
                     (rdm-review-test--comment 3 "Gone text" 60 69 :state "drifted")
                     (rdm-review-test--comment
                      4 "current" (string-bytes "A much longer new intro — ünïcode.\nSecond line with target words here.\nA drifting phrase!\nX 😀 ")
                      (string-bytes "A much longer new intro — ünïcode.\nSecond line with target words here.\nA drifting phrase!\nX 😀 current")
                      :body "current"))))
      (setq rdm-review-test--current new
            rdm-review-test--originals (list (cons "c0" old)))
      (with-current-buffer (rdm-open-item rdm-review-test--task)
        (let ((one (rdm-review-test--overlay-of 1))
              (two (rdm-review-test--overlay-of 2))
              (four (rdm-review-test--overlay-of 4)))
          (should (equal (rdm-review-test--text one) "target words"))
          (should (equal (rdm-review-test--faces one) '(rdm-review-resolved)))
          (should (equal (rdm-review-test--text two) "drifting"))
          (should (equal (rdm-review-test--faces two) '(rdm-review-drifted)))
          (should (equal (rdm-review-test--text four) "current")))
        (should-not (rdm-review-test--overlay-of 3))
        (should (string-match-p "#3 open  un-placed (drifted) \"Gone text\""
                                (rdm-review-test--section)))
        ;; The `created_commit' body is read once, with --at.
        (should (= 1 (seq-count (lambda (argv) (member "--at=c0" argv))
                                (rdm-mock-argvs calls))))))))

(ert-deftest rdm-review/repeated-quote-chosen-by-context ()
  "A quote occurring twice in a changed body is placed by its anchor's
context, else nearest its original offset."
  (let ((old "x AA y\nz AA w\n")
        (new "new line\nx AA y\nz AA w\n"))
    (rdm-review-test--with
        (list (rdm-review-test--review
               "r1" rdm-review-test--task-target
               (list (rdm-review-test--comment 1 "AA" 9 11 :prefix "z " :suffix " w")
                     (rdm-review-test--comment 2 "AA" 2 4))))
      (setq rdm-review-test--current new
            rdm-review-test--originals (list (cons "c0" old)))
      (with-current-buffer (rdm-open-item rdm-review-test--task)
        (let ((start (car (rdm-view-body-region))))
          (should (= (- (overlay-start (rdm-review-test--overlay-of 1)) start)
                     (string-search "AA w" new)))
          (should (= (- (overlay-start (rdm-review-test--overlay-of 2)) start)
                     (string-search "AA" new))))))))

(ert-deftest rdm-review/multibyte-ranges ()
  "Byte ranges over 2-, 3- and 4-byte characters cover exactly the quote;
a range starting inside a character or past the end is un-placed."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "naïve" 10 16 :body "current")
                   (rdm-review-test--comment 2 "—" 6 9 :body "current")
                   (rdm-review-test--comment 3 "日本語 text" 72 86 :body "current")
                   (rdm-review-test--comment 4 "😀 smile" 102 112 :body "current")
                   (rdm-review-test--comment 5 "bad" 7 9 :body "current")
                   (rdm-review-test--comment 6 "past" 110 999 :body "current"))))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (should (equal (rdm-review-test--text (rdm-review-test--overlay-of 1)) "naïve"))
      (should (equal (rdm-review-test--text (rdm-review-test--overlay-of 2)) "—"))
      (should (equal (rdm-review-test--text (rdm-review-test--overlay-of 3))
                     "日本語 text"))
      (should (equal (rdm-review-test--text (rdm-review-test--overlay-of 4))
                     "😀 smile"))
      (should-not (rdm-review-test--overlay-of 5))
      (should-not (rdm-review-test--overlay-of 6))
      (let ((section (rdm-review-test--section)))
        (should (string-match-p "#5 open  un-placed" section))
        (should (string-match-p "#6 open  un-placed" section))))))

(ert-deftest rdm-review/revert-rebuilds-overlays ()
  "No overlay survives a revert: each render rebuilds the layer."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (revert-buffer)
      (should (= (length (rdm-review-test--overlays)) 1))
      (setq rdm-review-test--reviews nil)
      (revert-buffer)
      (should-not (rdm-review-test--overlays))
      (should-not (string-match-p "\nReviews\n" (buffer-string))))))

(ert-deftest rdm-review/shown-states ()
  "Reviews outside `rdm-review-shown-states' are not shown."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))
             :state "dismissed"))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (should-not (rdm-review-test--overlays))
      (let ((rdm-review-shown-states '("dismissed")))
        (rdm-review-refresh)
        (should (= (length (rdm-review-test--overlays)) 1))))))

;;;; Failed reads

(defmacro rdm-review-test--capturing-messages (var &rest body)
  "Run BODY with `message' output pushed onto VAR, newest first."
  (declare (indent 1) (debug t))
  `(cl-letf (((symbol-function 'message)
              (lambda (fmt &rest args)
                (when fmt (push (apply #'format fmt args) ,var)))))
     ,@body))

(ert-deftest rdm-review/review-list-error-still-renders ()
  "A failing `review list' leaves the detail buffer rendered with its
body, no overlays and no Reviews section, and reports the error."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
    (setq rdm-review-test--failing '(list))
    (let (messages)
      (rdm-review-test--capturing-messages messages
        (let ((buf (rdm-open-item rdm-review-test--task)))
          (should (bufferp buf))
          (with-current-buffer buf
            (should (eq major-mode 'rdm-view-mode))
            (should (equal (rdm-view-body) rdm-review-test--body))
            (should-not (rdm-review-test--overlays))
            (should-not (string-match-p "\nReviews\n" (buffer-string)))
            ;; A revert takes the same path and still renders.
            (revert-buffer)
            (should (equal (rdm-view-body) rdm-review-test--body))
            (should-not (rdm-review-test--overlays)))))
      (should (seq-some (lambda (m)
                          (string-match-p
                           "rdm: reviews of .*unrecognized subcommand 'list'" m))
                        messages)))))

(ert-deftest rdm-review/original-read-error-falls-back-to-quote ()
  "A failing `show --at' read leaves the original unknown: an `original'
anchor whose quote is still displayed is placed by quote search, one
whose quote is gone is listed un-placed, and nothing is signalled."
  (let ((new "A much longer new intro.\nSecond line with target words here.\n"))
    (rdm-review-test--with
        (list (rdm-review-test--review
               "r1" rdm-review-test--task-target
               (list (rdm-review-test--comment 1 "target words" 22 34)
                     (rdm-review-test--comment 2 "Gone text" 40 49))))
      (setq rdm-review-test--current new
            rdm-review-test--originals
            (list (cons "c0" "Old.\nSecond line with target words here.\n"))
            rdm-review-test--failing '(at))
      (with-current-buffer (rdm-open-item rdm-review-test--task)
        (should (equal (rdm-view-body) new))
        (let ((one (rdm-review-test--overlay-of 1)))
          (should one)
          (should (equal (rdm-review-test--text one) "target words"))
          (should (equal (rdm-review-test--faces one) '(rdm-review-resolved))))
        (should-not (rdm-review-test--overlay-of 2))
        (should (string-match-p "#2 open  un-placed (resolved) \"Gone text\""
                                (rdm-review-test--section)))
        ;; The failing read was attempted.
        (should (seq-some (lambda (argv) (member "--at=c0" argv))
                          (rdm-mock-argvs calls)))))))

;;;; Resolution

(defun rdm-review-test--git-repo (message)
  "A temp git repository with one commit titled MESSAGE; return its dir."
  (let ((dir (file-name-as-directory (make-temp-file "rdm-review-git" t))))
    (let ((default-directory dir))
      (process-lines "git" "init" "-q")
      (process-lines "git" "-c" "user.name=t" "-c" "user.email=t@example.com"
                     "commit" "-q" "--allow-empty" "-m" message))
    dir))

(defun rdm-review-test--head (dir)
  "The HEAD commit of the repository DIR."
  (let ((default-directory dir))
    (car (process-lines "git" "rev-parse" "HEAD"))))

(ert-deftest rdm-review/resolve-argv-and-plan-head ()
  "gA marks the comment at point with --comment/--status/--reply and an
applied commit defaulting to the plan repo's HEAD, not the buffer's
repository's; no --no-edit.  The view is refreshed and the write hook runs."
  (let* ((plan (rdm-review-test--git-repo "plan"))
         (code (rdm-review-test--git-repo "code"))
         (hooked 0)
         (rdm-after-write-hook (list (lambda () (cl-incf hooked))))
         prefill)
    (should-not (equal (rdm-review-test--head plan) (rdm-review-test--head code)))
    (rdm-review-test--with
        (list (rdm-review-test--review
               "r1" rdm-review-test--task-target
               (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
      (cl-letf (((symbol-function 'rdm-context)
                 (lambda () (list :root plan :project "p" :rdm-bin "rdm")))
                ((symbol-function 'completing-read)
                 (lambda (&rest _) "addressed"))
                ((symbol-function 'read-string)
                 (lambda (prompt &optional initial &rest _)
                   (if (string-prefix-p "Reply" prompt)
                       "-- fixed"
                     (setq prefill initial)))))
        (with-current-buffer (rdm-open-item rdm-review-test--task)
          (setq default-directory code)
          (goto-char (overlay-start (rdm-review-test--overlay-of 1)))
          (let ((before (length calls)))
            (call-interactively #'rdm-review-resolve)
            (should (equal prefill (rdm-review-test--head plan)))
            (should (equal (car (nth before calls))
                           (list "review" "update" "r1" "--comment=1"
                                 "--status=addressed" "--reply=-- fixed"
                                 (concat "--applied-commit="
                                         (rdm-review-test--head plan))
                                 "--project" "p")))
            ;; The view re-read the item and its reviews.
            (should (seq-find (lambda (argv) (and (equal (car argv) "review")
                                                  (equal (cadr argv) "list")))
                              (mapcar #'car (nthcdr (1+ before) calls)))))
          (should (= hooked 1)))))))

(ert-deftest rdm-review/resolve-wont-fix-without-reply ()
  "wont-fix with an empty reply sends neither --reply nor --applied-commit."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (let ((entry (overlay-get (rdm-review-test--overlay-of 1) 'rdm-review-entry))
            (before (length calls)))
        (rdm-review-resolve entry "wont-fix" "" nil)
        (should (equal (car (nth before calls))
                       '("review" "update" "r1" "--comment=1" "--status=wont-fix"
                         "--project" "p")))))))

(ert-deftest rdm-review/resolve-in-revision-buffer ()
  "In a detail buffer opened with --at the layer is placed against that
revision's body, and a resolve re-reads the reviews there too."
  (let ((old "Old.\nSecond line with target words here.\n"))
    (rdm-review-test--with
        (list (rdm-review-test--review
               "r1" rdm-review-test--task-target
               (list (rdm-review-test--comment 1 "target words" 22 34))))
      (setq rdm-review-test--originals (list (cons "c0" old)))
      (with-current-buffer (rdm-open-item rdm-review-test--task "c0")
        (should (equal (rdm-view-body) old))
        (let ((ov (rdm-review-test--overlay-of 1)))
          (should (equal (rdm-review-test--text ov) "target words"))
          (should (= (- (overlay-start ov) (car (rdm-view-body-region)))
                     (string-search "target words" old)))
          (should (equal (rdm-review-test--faces ov) '(rdm-review-resolved))))
        (setq rdm-review-test--reviews
              (list (rdm-review-test--review
                     "r1" rdm-review-test--task-target
                     (list (rdm-review-test--comment 1 "target words" 22 34
                                                     :status "addressed")))))
        (let ((entry (overlay-get (rdm-review-test--overlay-of 1)
                                  'rdm-review-entry))
              (before (length calls)))
          (rdm-review-resolve entry "addressed" "" nil)
          (should (seq-find (lambda (argv) (and (equal (car argv) "review")
                                                (equal (cadr argv) "list")))
                            (mapcar #'car (nthcdr (1+ before) calls)))))
        (let ((ov (rdm-review-test--overlay-of 1)))
          (should (equal (rdm-review-test--text ov) "target words"))
          (should (equal (rdm-review-test--faces ov)
                         '(rdm-review-closed rdm-review-resolved))))))))

(ert-deftest rdm-review/close-dismiss ()
  "Close and dismiss thread --state; a refused close shows rdm's message."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (goto-char (overlay-start (rdm-review-test--overlay-of 1)))
      (let ((before (length calls)))
        (call-interactively #'rdm-review-close)
        (should (equal (car (nth before calls))
                       '("review" "update" "r1" "--state=addressed"
                         "--project" "p"))))
      (goto-char (point-min))
      (search-forward "r1  alice")
      (let ((before (length calls)))
        (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
          (call-interactively #'rdm-review-dismiss))
        (should (equal (car (nth before calls))
                       '("review" "update" "r1" "--state=dismissed"
                         "--project" "p"))))
      (let ((before (length calls)))
        (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) nil)))
          (should-not (rdm-review-dismiss (car rdm-review-test--reviews))))
        (should (= (length calls) before)))
      (setq rdm-review-test--update-error
            "error: failed to update review: review 'r1' has 2 open comment(s) — resolve them")
      (should (string-match-p
               "has 2 open comment(s)"
               (cadr (should-error (rdm-review-close (car rdm-review-test--reviews))
                                   :type 'user-error)))))))

;;;; Requests queue and revise

(ert-deftest rdm-review/requests-picker ()
  "A picked request opens its target item; plan and change rows are
offered without erroring the picker, and refuse to open."
  (rdm-review-test--with nil
    (setq rdm-review-test--requests
          (list (rdm-review-test--review
                 "r-phase" '(:kind "phase" :roadmap "r1" :stem "phase-2-two")
                 (list (rdm-review-test--comment 1 "café" 17 22 :body "current")))
                (rdm-review-test--review "r-plan" '(:kind "plan" :slug "pl") nil)
                (rdm-review-test--review "r-change" '(:kind "change" :head "abc") nil)))
    (setq rdm-review-test--reviews (list (car rdm-review-test--requests)))
    (let ((cands (rdm-review-request-candidates)))
      (should (equal (mapcar #'substring-no-properties cands)
                     '("phase/r1/phase-2-two  r-phase" "plan/pl  r-plan"
                       "change/abc  r-change")))
      (should (string-match-p "request-changes  1 open"
                              (rdm-pick-annotate (car cands)))))
    (cl-letf (((symbol-function 'completing-read)
               (lambda (&rest _) "phase/r1/phase-2-two  r-phase")))
      (with-current-buffer (rdm-review-requests)
        (should (eq (plist-get rdm-view--item :kind) 'phase))
        (should (equal (plist-get rdm-view--item :roadmap) "r1"))
        (should (equal (plist-get rdm-view--item :stem) "phase-2-two"))
        (should (member '("review" "list" "--on=phase/r1/phase-2-two"
                          "--project" "p" "--format" "json")
                        (rdm-mock-argvs calls)))
        (should (looking-at-p "café"))))
    (cl-letf (((symbol-function 'completing-read)
               (lambda (&rest _) "plan/pl  r-plan")))
      (should-error (rdm-review-requests) :type 'user-error))))

(ert-deftest rdm-review/revise-handoff ()
  "Revise calls `rdm-review-revise-function' with the plist; the default
copies the command and runs nothing."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (goto-char (overlay-start (rdm-review-test--overlay-of 1)))
      (let ((before (length calls)))
        (should (equal (call-interactively #'rdm-review-revise) "/rdm:revise r1"))
        (should (equal (car kill-ring) "/rdm:revise r1"))
        (should (= (length calls) before)))
      (let* (got
             (rdm-review-revise-function (lambda (plist) (setq got plist) 'handed))
             (rdm-review-revise-command-format "/rdm-revise %s"))
        (should (eq (rdm-review-revise (car rdm-review-test--reviews)) 'handed))
        (should (equal got '(:review-id "r1" :target "task/t1" :project "p"
                                        :command "/rdm-revise r1")))))))

(ert-deftest rdm-review/show-review ()
  "Show reads `review show' and displays its comments."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
    (let ((buf (rdm-review-show (car rdm-review-test--reviews))))
      (unwind-protect
          (with-current-buffer buf
            (should (string-match-p "Target: task/t1" (buffer-string)))
            (should (string-match-p "> café\nComment 1\\." (buffer-string)))
            (should (member '("review" "show" "r1" "--project" "p"
                              "--format" "json")
                            (rdm-mock-argvs calls))))
        (kill-buffer buf)))))

;;;; Authoring

(ert-deftest rdm-review/occurrences-non-overlapping ()
  "Occurrences are counted without overlap, as `--occurrence' documents;
a start inside an overlapping repeat is refused."
  (should (equal (rdm-review-occurrence-starts "aaaa" "aa") '(0 2)))
  (should (equal (rdm-review-occurrence-starts "aaaaa" "aa") '(0 2)))
  (should-not (rdm-review-occurrence-starts "abc" ""))
  (should (= (rdm-review-occurrence "aaaa" "aa" 2) 2))
  (should (= (rdm-review-occurrence "aaaa" "aa" 0) 1))
  (should-not (rdm-review-occurrence "x once y" "once" 2))
  (should-error (rdm-review-occurrence "aaaa" "aa" 1) :type 'user-error))

(defconst rdm-review-test--authoring-body
  "Intro: open the door.\nrepeat me here\nand repeat me there\naaaa end\n"
  "A body with a repeated phrase, an overlapping run, and \"open\" (which
the header's Status line and the Reviews section also show).")

(defun rdm-review-test--draft (&rest keys)
  "An own draft review r1 of task t1 created at c0; KEYS override."
  (apply #'rdm-review-test--review "r1" rdm-review-test--task-target nil
         (append keys (list :state "draft" :verdict nil :author "me"
                            :created "2026-09-26T09:00:00Z"))))

(defmacro rdm-review-test--authoring (&rest body)
  "Run BODY in the detail buffer of task t1 with an own draft r1 at c0.
The current body is `rdm-review-test--authoring-body' with a line more,
so the draft's text differs from the current one."
  (declare (indent 0) (debug t))
  `(rdm-review-test--with (list (rdm-review-test--draft))
     (setq rdm-review-test--current
           (concat rdm-review-test--authoring-body "Added since.\n")
           rdm-review-test--originals
           (list (cons "c0" rdm-review-test--authoring-body)))
     (with-current-buffer (rdm-open-item rdm-review-test--task)
       ,@body)))

(defun rdm-review-test--body-range (text &optional nth)
  "The buffer range (BEG . END) of the NTH (1-based) TEXT in the body."
  (let* ((start (car (rdm-view-body-region)))
         (offset (nth (1- (or nth 1))
                      (rdm-review-occurrence-starts (rdm-view-body) text))))
    (cons (+ start offset) (+ start offset (length text)))))

(defun rdm-review-test--comment-on (range text)
  "Comment TEXT on RANGE through the package's compose buffer."
  (let ((compose (rdm-review-comment-region (car range) (cdr range))))
    (with-current-buffer compose
      (insert text)
      (rdm-compose-submit))))

(defun rdm-review-test--calls-of (calls &rest words)
  "The argvs among CALLS that start with WORDS."
  (seq-filter (lambda (argv) (equal (seq-take argv (length words)) words))
              (rdm-mock-argvs calls)))

(ert-deftest rdm-review/open-draft-renders-at-created-commit ()
  "An item with an own open draft renders at its `created_commit' and says
so; the draft lookup passes the author the package starts drafts with."
  (rdm-review-test--authoring
    (should (equal rdm-view--at "c0"))
    (should (equal (rdm-view-body) rdm-review-test--authoring-body))
    (should (string-match-p "Reviewing as of c0" (format "%s" header-line-format)))
    (should (string-match-p "draft r1" (format "%s" header-line-format)))
    (should (string-match-p "^Revision: c0$" (buffer-string)))
    (should (member '("review" "list" "--on=task/t1" "--state=draft" "--author=me"
                      "--project" "p" "--format" "json")
                    (rdm-mock-argvs calls)))
    (should (member "--at=c0" (car (last (rdm-review-test--calls-of
                                          calls "task" "show"))))))
  ;; Someone else's draft is not followed.
  (rdm-review-test--with (list (rdm-review-test--draft :author "bob"))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (should-not rdm-view--at)
      (should-not header-line-format))))

(ert-deftest rdm-review/region-comment-quote-and-occurrence ()
  "A region becomes --quote verbatim; a repeated phrase gets the
occurrence of the selected copy, a single one no --occurrence."
  (rdm-review-test--authoring
    (rdm-review-test--comment-on (rdm-review-test--body-range "repeat me" 2) "Second.")
    (rdm-review-test--comment-on (rdm-review-test--body-range "repeat me" 1) "First.")
    (rdm-review-test--comment-on (rdm-review-test--body-range "here\nand") "Span.")
    (should (equal (rdm-review-test--calls-of calls "review" "comment")
                   '(("review" "comment" "r1" "--quote=repeat me" "--occurrence=2"
                      "--body=Second." "--project" "p" "--no-edit")
                     ("review" "comment" "r1" "--quote=repeat me" "--occurrence=1"
                      "--body=First." "--project" "p" "--no-edit")
                     ("review" "comment" "r1" "--quote=here\nand"
                      "--body=Span." "--project" "p" "--no-edit"))))
    ;; No new draft was started: the open one was used.
    (should-not (rdm-review-test--calls-of calls "review" "start"))
    ;; The layer was re-read after each comment.
    (should (equal rdm-view--at "c0"))))

(ert-deftest rdm-review/region-occurrence-counts-body-only ()
  "\"open\" also shows in the Status line and the Reviews section; the
occurrence counts the body alone."
  (rdm-review-test--authoring
    (let ((region (rdm-view-body-region)))
      (should (string-match-p "Status: open" (buffer-substring 1 (car region))))
      (should (string-match-p "open" (buffer-substring (cdr region) (point-max)))))
    (rdm-review-test--comment-on (rdm-review-test--body-range "open") "Once.")
    (should (equal (car (rdm-review-test--calls-of calls "review" "comment"))
                   '("review" "comment" "r1" "--quote=open" "--body=Once."
                     "--project" "p" "--no-edit")))))

(ert-deftest rdm-review/region-refusals-issue-no-call ()
  "A region reaching outside the body, an empty one, and one starting
inside an overlapping repeat are refused before any rdm call."
  (rdm-review-test--authoring
    (let ((before (length calls))
          (body (rdm-view-body-region)))
      (should-error (rdm-review-comment-region (1- (car body)) (+ (car body) 4))
                    :type 'user-error)
      (should-error (rdm-review-comment-region (- (cdr body) 3) (1+ (cdr body)))
                    :type 'user-error)
      (should-error (rdm-review-comment-region (car body) (car body))
                    :type 'user-error)
      (let ((run (rdm-review-test--body-range "aaaa")))
        (should-error (rdm-review-comment-region (1+ (car run)) (+ 3 (car run)))
                      :type 'user-error))
      (should (= (length calls) before)))))

(ert-deftest rdm-review/document-comment-has-no-quote ()
  "C files a whole-document comment: no --quote, no --occurrence; an empty
comment is refused and its compose buffer kept."
  (rdm-review-test--authoring
    (let ((compose (rdm-review-comment-document)))
      (with-current-buffer compose
        (should-error (rdm-compose-submit) :type 'user-error)
        (should (buffer-live-p compose))
        (insert "Whole thing.")
        (rdm-compose-submit)))
    (should (equal (rdm-review-test--calls-of calls "review" "comment")
                   '(("review" "comment" "r1" "--body=Whole thing."
                      "--project" "p" "--no-edit"))))))

(ert-deftest rdm-review/first-selection-on-other-text-discarded ()
  "Without a draft, the first comment starts one and re-renders at its
`created_commit'; a selection made on text that differs is discarded,
not submitted.  A selection on the draft's text then lands."
  (rdm-review-test--with nil
    (setq rdm-review-test--current "Committed.\nStaged only text.\n"
          rdm-review-test--originals (list (cons "c-head" "Committed.\n")))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (should-not rdm-view--at)
      (let ((range (rdm-review-test--body-range "Staged only")))
        (should (string-match-p
                 "select it again"
                 (cadr (should-error (rdm-review-comment-region (car range) (cdr range))
                                     :type 'user-error)))))
      (should (equal (rdm-review-test--calls-of calls "review" "start")
                     '(("review" "start" "--on=task/t1" "--author=me" "--no-edit"
                        "--project" "p" "--format" "json"))))
      (should-not (rdm-review-test--calls-of calls "review" "comment"))
      (should (equal rdm-view--at "c-head"))
      (should (equal (rdm-view-body) "Committed.\n"))
      (should (string-match-p "Reviewing as of c-head"
                              (format "%s" header-line-format)))
      (rdm-review-test--comment-on (rdm-review-test--body-range "Committed") "Ok.")
      (should (equal (rdm-review-test--calls-of calls "review" "comment")
                     '(("review" "comment" "r-new" "--quote=Committed" "--body=Ok."
                        "--project" "p" "--no-edit"))))
      (should (= 1 (length (rdm-review-test--calls-of calls "review" "start")))))))

(ert-deftest rdm-review/submit-threads-verdict ()
  "Submit offers the verdicts `describe review' declares and threads the
chosen one; declining the confirmation writes nothing; afterwards the
buffer is back at the current revision with the submitted review."
  (dolist (verdict '("approve" "request-changes" "comment" "needs-discussion"))
    (rdm-review-test--authoring
      (setq rdm-review-test--verdicts
            '("approve" "request-changes" "comment" "needs-discussion"))
      (rdm-context-reset)
      (let (offered)
        (cl-letf (((symbol-function 'completing-read)
                   (lambda (_prompt collection &rest _)
                     (setq offered collection) verdict))
                  ((symbol-function 'y-or-n-p) (lambda (&rest _) nil)))
          (let ((before (length calls)))
            (should-not (call-interactively #'rdm-review-submit))
            (should-not (rdm-review-test--calls-of (nthcdr before calls)
                                                   "review" "submit"))))
        (should (equal offered rdm-review-test--verdicts))
        (cl-letf (((symbol-function 'completing-read) (lambda (&rest _) verdict))
                  ((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
          (call-interactively #'rdm-review-submit))
        (should (equal (rdm-review-test--calls-of calls "review" "submit")
                       (list (list "review" "submit" "r1"
                                   (concat "--verdict=" verdict)
                                   "--project" "p" "--no-edit"))))
        (should-not rdm-view--at)
        (should-not header-line-format)
        (should (equal (rdm-view-body) rdm-review-test--current))
        (should (string-match-p (concat "r1  me  submitted " verdict)
                                (rdm-review-test--section)))))))

(ert-deftest rdm-review/submit-without-draft-refused ()
  "Submitting with no draft of yours is refused before any prompt."
  (rdm-review-test--with nil
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (cl-letf (((symbol-function 'completing-read)
                 (lambda (&rest _) (error "Unexpected prompt"))))
        (should-error (call-interactively #'rdm-review-submit) :type 'user-error)))
    (should-not (rdm-review-test--calls-of calls "review" "submit"))))

;;;; The transient

(defun rdm-review-test--dispatch-suffixes (item)
  "The (KEY . COMMAND) suffixes `rdm-dispatch' offers for ITEM."
  (unwind-protect
      (progn (rdm-dispatch-item item)
             (mapcar (lambda (obj) (cons (oref obj key) (oref obj command)))
                     transient--suffixes))
    (transient-quit-all)))

(ert-deftest rdm-review/dispatch-review-group ()
  "The Review group is offered with an item; its keys never collide with
generated status keys."
  (rdm-transient-test--with-stub nil
    (let ((none (rdm-review-test--dispatch-suffixes nil))
          (task (rdm-review-test--dispatch-suffixes rdm-transient-test--task)))
      (pcase-dolist (`(,key . ,command)
                     '(("Rs" . rdm-review-dispatch-show)
                       ("Rq" . rdm-review-requests)
                       ("Ra" . rdm-review-dispatch-close)
                       ("Rd" . rdm-review-dispatch-dismiss)
                       ("Rv" . rdm-review-dispatch-revise)
                       ("Rn" . rdm-review-dispatch-start)
                       ("Rc" . rdm-review-dispatch-comment)
                       ("RS" . rdm-review-dispatch-submit)))
        (should (equal (cdr (assoc key task)) command))
        (should (commandp command))
        (should-not (assoc key none)))))
  (let ((keys (rdm-dispatch--status-keys '("Review" "Ready" "reviewed"))))
    (should-not (member "R" keys))))

;;;; Evil

(ert-deftest rdm-evil/review-keys ()
  "K, ]c, [c and gA reach the review commands in normal state."
  :tags '(evil)
  (rdm-test-with-soft-dep 'evil
    (with-temp-buffer
      (rdm-view-mode)
      (evil-local-mode 1)
      (should (eq evil-state 'normal))
      (should (eq (key-binding (kbd "K")) #'rdm-review-comment-at-point))
      (should (eq (key-binding (kbd "]c")) #'rdm-review-next-comment))
      (should (eq (key-binding (kbd "[c")) #'rdm-review-previous-comment))
      (should (eq (key-binding (kbd "gA")) #'rdm-review-resolve))
      (evil-local-mode -1))))

(ert-deftest rdm-review/vanilla-keys ()
  "Without evil, the review commands have their own keys."
  (with-temp-buffer
    (rdm-view-mode)
    (should (eq (lookup-key rdm-view-mode-map (kbd "K")) #'rdm-review-comment-at-point))
    (should (eq (lookup-key rdm-view-mode-map (kbd "M-n")) #'rdm-review-next-comment))
    (should (eq (lookup-key rdm-view-mode-map (kbd "M-p")) #'rdm-review-previous-comment))
    (should (eq (lookup-key rdm-view-mode-map (kbd "A")) #'rdm-review-resolve))
    (should (eq (lookup-key rdm-view-mode-map (kbd "c")) #'rdm-review-comment-region))
    (should (eq (lookup-key rdm-view-mode-map (kbd "C")) #'rdm-review-comment-document))
    (should (eq (lookup-key rdm-view-mode-map (kbd "S")) #'rdm-review-submit))))

(ert-deftest rdm-evil/review-authoring-keys ()
  "Visual c comments on the selection; normal C comments on the whole
document and gS submits the draft."
  :tags '(evil)
  (rdm-test-with-soft-dep 'evil
    ;; An earlier test's transient may still hold
    ;; `overriding-terminal-local-map' (binding its own `C'); these are
    ;; the buffer's own bindings.
    (with-temp-buffer
      (let ((overriding-terminal-local-map nil))
        (rdm-view-mode)
        (let ((inhibit-read-only t)) (insert "Some body text.\n"))
        (evil-local-mode 1)
        (should (eq evil-state 'normal))
        (should (eq (key-binding (kbd "C")) #'rdm-review-comment-document))
        (should (eq (key-binding (kbd "gS")) #'rdm-review-submit))
        (goto-char (point-min))
        (evil-visual-char)
        (should (eq evil-state 'visual))
        (should (eq (key-binding (kbd "c")) #'rdm-review-comment-region))
        (evil-exit-visual-state)
        (evil-local-mode -1)))))

;;;; Against the real binary

(defun rdm-review-test--rdm (bin &rest args)
  "Run the real BIN with ARGS in this buffer's environment; return stdout."
  (rdm-run-sync (cons bin args)))

(ert-deftest rdm-review/real-overlay-drift-resolve ()
  "A review seeded with the real binary renders resolved over multibyte
text, follows an edit that moves the quote, lists the comment as drifted
once the quote is gone, and resolving it through the package is what
`review show' reports; the requests queue offers it."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project))
                  process-environment))
    (setq rdm-bin bin)
    (let* ((rdm-session "emacs-real-review-test")
           (slug "emacs-review-real-task")
           (item (list :kind 'task :slug slug))
           (kill-ring nil)
           id)
      (unwind-protect
          (progn
            (rdm-call-sync 'task-create slug :title "Reviewed from Emacs"
                           :stdin rdm-review-test--body)
            (rdm-call-sync 'commit :message "emacs: seed a reviewed task")
            (let ((out (rdm-review-test--rdm bin "review" "start" "--on"
                                             (concat "task/" slug) "--no-edit"
                                             "--project" project)))
              (should (string-match "Started review '\\([^']+\\)'" out))
              (setq id (match-string 1 out)))
            (rdm-review-test--rdm bin "review" "comment" id "--quote" "日本語 text"
                                  "--body" "Say it in English." "--no-edit"
                                  "--project" project)
            (rdm-review-test--rdm bin "review" "submit" id "--verdict"
                                  "request-changes" "--no-edit"
                                  "--project" project)
            (with-current-buffer (rdm-open-item item)
              (let ((ov (car (rdm-review-test--overlays))))
                (should ov)
                (should (equal (rdm-review-test--text ov) "日本語 text"))
                (should (equal (rdm-review-test--faces ov) '(rdm-review-resolved))))
              ;; An edit elsewhere moves the quote's offset: still resolved.
              (rdm-call-sync 'task-update slug
                             :body (concat "Short.\n" rdm-review-test--body))
              (revert-buffer)
              (let ((ov (car (rdm-review-test--overlays))))
                (should (equal (rdm-review-test--text ov) "日本語 text"))
                (should (equal (rdm-review-test--faces ov) '(rdm-review-resolved))))
              ;; A staged edit removing the quote drifts it.
              (rdm-call-sync 'task-update slug :body "All rewritten.\n")
              (revert-buffer)
              (should-not (rdm-review-test--overlays))
              (should (string-match-p "#1 open  un-placed (drifted)"
                                      (rdm-review-test--section)))
              (should (seq-find (lambda (c)
                                  (equal (plist-get (rdm-item-from c) :review-id) id))
                                (rdm-review-request-candidates)))
              (goto-char (point-min))
              (search-forward "#1 open")
              (cl-letf (((symbol-function 'completing-read)
                         (lambda (&rest _) "addressed"))
                        ((symbol-function 'read-string)
                         (lambda (prompt &optional initial &rest _)
                           (if (string-prefix-p "Reply" prompt)
                               "Rewritten."
                             initial))))
                (call-interactively #'rdm-review-resolve))
              (let* ((shown (rdm-call-sync 'review-show id))
                     (comment (aref (plist-get shown :comments) 0))
                     (head (let ((default-directory (file-name-as-directory root)))
                             (car (process-lines "git" "rev-parse" "HEAD")))))
                (should (equal (plist-get comment :status) "addressed"))
                (should (equal (plist-get comment :reply) "Rewritten."))
                (should (equal (plist-get comment :applied_commit) head)))
              (should (string-match-p "#1 addressed" (rdm-review-test--section)))))
        (ignore-errors (rdm-call-sync 'commit :message "emacs: review round trip"))
        (dolist (b (buffer-list))
          (when (eq (buffer-local-value 'major-mode b) 'rdm-view-mode)
            (kill-buffer b)))))))

(defmacro rdm-review-test--with-real-author (session &rest body)
  "Run BODY against the real binary as author \"emacs-author\" in SESSION.
Whatever BODY stages is committed afterwards and detail buffers killed."
  (declare (indent 1) (debug t))
  `(rdm-test-with-real
     (setq process-environment
           (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project))
                   process-environment))
     (setq rdm-bin bin)
     (let ((rdm-session ,session)
           (rdm-review-author "emacs-author")
           (kill-ring nil))
       (unwind-protect (progn ,@body)
         (ignore-errors (rdm-call-sync 'commit :message "emacs: review authoring"))
         (dolist (b (buffer-list))
           (when (memq (buffer-local-value 'major-mode b)
                       '(rdm-view-mode rdm-compose-mode))
             (kill-buffer b)))))))

(defun rdm-review-test--plan-head (root)
  "The HEAD commit of the plan repo ROOT."
  (let ((default-directory (file-name-as-directory root)))
    (car (process-lines "git" "rev-parse" "HEAD"))))

(ert-deftest rdm-review/real-first-selection-anchors-at-created-commit ()
  "With a staged body edit, the first selection starts a draft whose
`created_commit' is the plan repo's HEAD: the buffer re-renders at it
and says so, the selection made on the staged-only text is discarded
without a comment, and a selection on the committed text then lands."
  (rdm-review-test--with-real-author "emacs-real-author-staged"
    (let* ((slug "emacs-author-staged-task")
           (item (list :kind 'task :slug slug))
           (committed "Committed line one.\nCommitted line two.\n")
           (staged (concat committed "Staged only line.\n")))
      (rdm-call-sync 'task-create slug :title "Authored from Emacs" :stdin committed)
      (rdm-call-sync 'commit :message "emacs: seed an authored task")
      (rdm-call-sync 'task-update slug :body staged)
      (with-current-buffer (rdm-open-item item)
        (should (equal (rdm-view-body) staged))
        (should-not rdm-view--at)
        (let ((range (rdm-review-test--body-range "Staged only")))
          (should-error (rdm-review-comment-region (car range) (cdr range))
                        :type 'user-error))
        (let ((id (plist-get rdm-review--draft :id))
              (head (rdm-review-test--plan-head root)))
          (should id)
          (should (equal rdm-view--at head))
          (should (equal (rdm-view-body) committed))
          (should (string-match-p (concat "Reviewing as of " (substring head 0 12))
                                  (format "%s" header-line-format)))
          (should (string-match-p (concat "^Revision: " head) (buffer-string)))
          (let ((review (rdm-call-sync 'review-show id)))
            (should (equal (plist-get review :created_commit) head))
            (should (equal (plist-get review :author) "emacs-author"))
            (should (= 0 (length (plist-get review :comments)))))
          (rdm-review-test--comment-on (rdm-review-test--body-range "line two")
                                       "Say more.")
          (let ((comments (plist-get (rdm-call-sync 'review-show id) :comments)))
            (should (= 1 (length comments)))
            (should (equal (plist-get (plist-get (aref comments 0) :anchor) :quote)
                           "line two")))
          ;; The overlay layer shows the new comment on the draft's text.
          (let ((ov (rdm-review-test--overlay-of 1)))
            (should ov)
            (should (equal (rdm-review-test--text ov) "line two"))))))))

(ert-deftest rdm-review/real-repeat-anchor-and-request-changes ()
  "Commenting on the second of two copies of a string anchors there:
`review show' reports it resolved at that copy's byte offset.
Submitting request-changes through the package, with the verdict picked
from those core declares, lands in `review requests' and the requests
picker."
  (rdm-review-test--with-real-author "emacs-real-author-repeat"
    (let* ((slug "emacs-author-repeat-task")
           (item (list :kind 'task :slug slug))
           (body "café alpha one\nsecond alpha two\n"))
      (rdm-call-sync 'task-create slug :title "Repeated words" :stdin body)
      (rdm-call-sync 'commit :message "emacs: seed a repeated-words task")
      (with-current-buffer (rdm-open-item item)
        (let ((id (rdm-review-start)))
          (should (equal rdm-view--at (rdm-review-test--plan-head root)))
          (should (equal (rdm-view-body) body))
          (rdm-review-test--comment-on (rdm-review-test--body-range "alpha" 2)
                                       "This one.")
          (let* ((comment (aref (plist-get (rdm-call-sync 'review-show id) :comments) 0))
                 (res (plist-get comment :resolution))
                 (second (nth 1 (rdm-review-occurrence-starts body "alpha"))))
            (should (equal (plist-get res :state) "resolved"))
            (should (= (plist-get res :range_start)
                       (string-bytes (substring body 0 second))))
            (should (= (plist-get res :range_end)
                       (string-bytes (substring body 0 (+ second 5))))))
          (let (offered)
            (cl-letf (((symbol-function 'completing-read)
                       (lambda (_prompt collection &rest _)
                         (setq offered collection)
                         "request-changes"))
                      ((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
              (call-interactively #'rdm-review-submit))
            (should (member "request-changes" offered)))
          (let ((review (rdm-call-sync 'review-show id)))
            (should (equal (plist-get review :state) "submitted"))
            (should (equal (plist-get review :verdict) "request-changes")))
          (should (seq-find (lambda (r) (equal (plist-get r :id) id))
                            (rdm-call-sync 'review-requests)))
          (should (seq-find (lambda (c)
                              (equal (plist-get (rdm-item-from c) :review-id) id))
                            (rdm-review-request-candidates)))
          (should-not rdm-view--at)
          (should-not rdm-review--draft)
          (should (string-match-p (concat (regexp-quote id)
                                          "  emacs-author  submitted request-changes")
                                  (rdm-review-test--section))))))))

(provide 'rdm-review-test)
;;; rdm-review-test.el ends here
