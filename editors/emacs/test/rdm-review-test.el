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

(defvar rdm-review-test--comment-flags
  [(:value "addressed" :flag "applied-commit")]
  "The `value_flags' the mocked `describe review' declares for comments.")

(defvar rdm-review-test--new-id "r-new"
  "The id the mocked `review start' gives a new draft.")

(defvar rdm-review-test--head "c-head"
  "The `created_commit' the mocked `review start' stamps.")

(defvar rdm-review-test--resolved-author "env-me"
  "The author the mocked rdm resolves for `--mine' and an unflagged start.")

(defun rdm-review-test--closed (status)
  "The `closed' rdm reports for a comment STATUS."
  (if (member status '("addressed" "wont-fix")) t :false))

(defun rdm-review-test--find (id)
  "The mocked review ID, or nil."
  (seq-find (lambda (r) (equal (plist-get r :id) id)) rdm-review-test--reviews))

(defun rdm-review-test--update (id argv)
  "Apply the mocked `review update ID' ARGV; return the review JSON."
  (let ((review (or (rdm-review-test--find id)
                    (list :id id :state "submitted" :comments []
                          :target '(:kind "task" :slug "t1" :ref "task/t1"
                                   :item_ref "task/t1"))))
        (comment (rdm-review-test--flag argv "comment"))
        (status (rdm-review-test--flag argv "status"))
        (state (rdm-review-test--flag argv "state")))
    (when (and comment status)
      (seq-doseq (c (plist-get review :comments))
        (when (equal (format "%s" (plist-get c :id)) comment)
          (plist-put c :status status)
          (plist-put c :closed (rdm-review-test--closed status)))))
    (if state (plist-put review :state state) review)))

(defun rdm-review-test--flag-target (ref)
  "The target JSON of the reference REF (task/SLUG or phase/R/STEM)."
  (pcase (split-string ref "/")
    (`("task" ,slug) (list :kind "task" :slug slug :ref ref :item_ref ref))
    (`("phase" ,roadmap ,stem)
     (list :kind "phase" :roadmap roadmap :stem stem :ref ref
           :item_ref (concat roadmap "/" stem)))
    (`("roadmap" ,roadmap)
     (list :kind "roadmap" :roadmap roadmap :ref ref :item_ref roadmap))))

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
      (`(,(and kind (or "task" "phase")) "show" ,name . ,_)
       (let ((ref (if (equal kind "task")
                      (concat "task/" name)
                    (format "%s/%s" (rdm-review-test--flag argv "roadmap") name))))
         (list :title "T" :status "open" :ref ref
               :review_ref (if (equal kind "task") ref (concat "phase/" ref))
               :body (if at
                         (cdr (assoc at rdm-review-test--originals))
                       rdm-review-test--current))))
      (`("roadmap" "show" ,slug . ,_)
       (list :title "R" :body rdm-review-test--current
             :ref slug :review_ref (concat "roadmap/" slug)
             :phases (vector (list :number 1 :stem "phase-1-one"
                                   :ref (concat slug "/phase-1-one")
                                   :review_ref (format "phase/%s/phase-1-one" slug)
                                   :title "One" :status "done"))))
      (`("review" "list" . ,_)
       ;; Served in the order given: rdm's own (newest-first) order.
       (let ((state (rdm-review-test--flag argv "state"))
             (author (or (rdm-review-test--flag argv "author")
                         (and (member "--mine" argv)
                              rdm-review-test--resolved-author))))
         (vconcat (seq-filter
                   (lambda (r)
                     (and (or (not state) (equal (plist-get r :state) state))
                          (or (not author) (equal (plist-get r :author) author))))
                   rdm-review-test--reviews))))
      (`("describe" "review" . ,_)
       (list :name "review"
             :fields (vector '(:name "state" :enum_values ["draft" "submitted"])
                             (list :name "verdict"
                                   :enum_values (vconcat rdm-review-test--verdicts))
                             (list :name "comments.status"
                                   :enum_values ["open" "addressed" "wont-fix"]
                                   :terminal_values ["addressed" "wont-fix"]
                                   :value_flags rdm-review-test--comment-flags))))
      (`("review" "start" . ,_)
       (let ((review (rdm-review-test--review
                      rdm-review-test--new-id
                      (rdm-review-test--flag-target (rdm-review-test--flag argv "on"))
                      nil :state "draft" :verdict nil
                      :author (or (rdm-review-test--flag argv "author")
                                  rdm-review-test--resolved-author)
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
         (rdm-review-test--update id argv)))
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
         (rdm-review-test--comment-flags
          [(:value "addressed" :flag "applied-commit")])
         (rdm-review-test--new-id "r-new")
         (rdm-review-test--head "c-head")
         (rdm-review-test--resolved-author "env-me")
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
`:status' (\"open\"), `:doc', `:prefix', `:suffix', `:current' (a
\(START . END) byte pair into the current body, or nil for rdm's null;
default START..END) and `:anchor' (replaces the quote anchor)."
  (let ((current (if (plist-member keys :current)
                     (plist-get keys :current)
                   (cons start end))))
    (append
     (list :id id :status (or (plist-get keys :status) "open")
           :closed (rdm-review-test--closed (or (plist-get keys :status) "open"))
           :anchor (or (plist-get keys :anchor)
                       (list :anchor_type "text-quote" :quote quote
                             :prefix (or (plist-get keys :prefix) "")
                             :suffix (or (plist-get keys :suffix) "")))
           :body (format "Comment %s." id)
           :resolution (list :state (or (plist-get keys :state) "resolved")
                             :quote quote :range_start start :range_end end
                             :body (or (plist-get keys :body) "original")
                             :current (and current
                                           (list :range_start (car current)
                                                 :range_end (cdr current)))))
     (when (plist-get keys :doc)
       (list :doc (list :kind "phase" :stem (plist-get keys :doc)))))))

(defun rdm-review-test--line-comment (id lines snippet start end &rest keys)
  "A line-range comment ID on LINES (START-LINE . END-LINE) with SNIPPET.
START..END is its byte range in the `created_commit' body; KEYS are as
for `rdm-review-test--comment'."
  (apply #'rdm-review-test--comment id snippet start end
         :anchor (list :anchor_type "line-range"
                       :start_line (car lines) :end_line (cdr lines)
                       :snippet snippet)
         keys))

(defun rdm-review-test--review (id target comments &rest keys)
  "A review ID on TARGET (JSON) with COMMENTS; KEYS override fields."
  (append keys
          (list :id id :author "alice" :target target
                :state "submitted" :verdict "request-changes"
                :created_commit "c0" :body ""
                :comments (vconcat comments))))

(defconst rdm-review-test--task-target
  '(:kind "task" :slug "t1" :ref "task/t1" :item_ref "task/t1"))
(defconst rdm-review-test--task '(:kind task :slug "t1" :ref "task/t1"))

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
                   (list :id 2 :status "open" :closed :false :body "Whole doc."
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
             "r1" '(:kind "roadmap" :roadmap "r1" :ref "roadmap/r1" :item_ref "r1")
             (list (rdm-review-test--comment 1 "Intro" 0 5 :doc "phase-1-one")
                   (rdm-review-test--comment 2 "café" 17 22))))
    (setq rdm-review-test--originals (list (cons "c0" rdm-review-test--body)))
    (with-current-buffer (rdm-open-item '(:kind roadmap :roadmap "r1"))
      (should-not (rdm-review-test--overlay-of 1))
      (should (equal (rdm-review-test--text (rdm-review-test--overlay-of 2))
                     "café"))
      (should (string-match-p "#1 open  on phase phase-1-one"
                              (rdm-review-test--section))))))

(defconst rdm-review-test--old
  "Old.\nSecond line with target words here.\nA drifting phrase.\nGone text.\n"
  "A `created_commit' body the placement tests review.")

(defconst rdm-review-test--new
  "A much longer new intro — ünïcode.\nSecond line with target words here.\nA drifting phrase!\nX 😀 current.\n"
  "The current body the placement tests render.")

(defun rdm-review-test--bytes-before (body text)
  "The UTF-8 byte offset in BODY of the first TEXT."
  (string-bytes (substring body 0 (string-search text body))))

(defun rdm-review-test--placement-reviews ()
  "One review of task t1 at c0 whose comments place differently.
#1 resolved, `current' moved; #2 drifted, `current' still set; #3
drifted, `current' null; #4 a line range whose lines shifted (`current'
null); #5 a range indexing the current body (`body' current)."
  (let* ((old rdm-review-test--old)
         (new rdm-review-test--new)
         (at (lambda (body text)
               (let ((s (rdm-review-test--bytes-before body text)))
                 (cons s (+ s (string-bytes text)))))))
    (list (rdm-review-test--review
           "r1" rdm-review-test--task-target
           (list (rdm-review-test--comment
                  1 "target words" 22 34
                  :current (funcall at new "target words"))
                 (rdm-review-test--comment
                  2 "drifting" 43 51 :state "drifted"
                  :current (funcall at new "drifting"))
                 (rdm-review-test--comment 3 "Gone text" 60 69 :state "drifted"
                                           :current nil)
                 (rdm-review-test--line-comment
                  4 '(2 . 3) "Second line with target words here.\nA drifting phrase."
                  5 59 :state "drifted" :current nil)
                 (rdm-review-test--comment
                  5 "😀 current" (car (funcall at new "😀 current"))
                  (cdr (funcall at new "😀 current"))
                  :body "current"))))))

(defun rdm-review-test--at-calls (calls)
  "The argvs among CALLS that pass `--at'."
  (seq-filter (lambda (argv) (rdm-review-test--flag argv "at"))
              (rdm-mock-argvs calls)))

(ert-deftest rdm-review/current-buffer-places-from-current ()
  "A current-body buffer places each comment at its resolution's
`current' range (over multibyte text), keeps the drifted face, and lists
a comment whose `current' is null un-placed -- a shifted line range
included.  Nothing else is read: no `show --at'."
  (rdm-review-test--with (rdm-review-test--placement-reviews)
    (setq rdm-review-test--current rdm-review-test--new
          rdm-review-test--originals (list (cons "c0" rdm-review-test--old)))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (should-not rdm-view--at)
      (let ((one (rdm-review-test--overlay-of 1))
            (two (rdm-review-test--overlay-of 2))
            (five (rdm-review-test--overlay-of 5)))
        (should (equal (rdm-review-test--text one) "target words"))
        (should (equal (rdm-review-test--faces one) '(rdm-review-resolved)))
        (should (equal (rdm-review-test--text two) "drifting"))
        (should (equal (rdm-review-test--faces two) '(rdm-review-drifted)))
        (should (equal (rdm-review-test--text five) "😀 current")))
      (should-not (rdm-review-test--overlay-of 3))
      (should-not (rdm-review-test--overlay-of 4))
      (let ((section (rdm-review-test--section)))
        (should (string-match-p "#3 open  un-placed (drifted) \"Gone text\"" section))
        (should (string-match-p
                 "#4 open  un-placed (drifted) lines 2-3 \"Second line with" section)))
      (goto-char (point-min))
      (search-forward "#4 open")
      (should (string-match-p "un-placed (drifted), lines 2-3"
                              (rdm-review-comment-at-point))))
    (should-not (rdm-review-test--at-calls calls))))

(ert-deftest rdm-review/created-commit-buffer-places-from-original ()
  "A buffer showing the review's `created_commit' places each comment at
its `original' range, drifted ones included (the shifted line range sits
on its original lines); a range indexing the current body is listed.
The only `show --at' read is the render itself."
  (rdm-review-test--with (rdm-review-test--placement-reviews)
    (setq rdm-review-test--current rdm-review-test--new
          rdm-review-test--originals (list (cons "c0" rdm-review-test--old)))
    (with-current-buffer (rdm-open-item rdm-review-test--task "c0")
      (should (equal (rdm-view-body) rdm-review-test--old))
      (let ((start (car (rdm-view-body-region))))
        (dolist (want '((1 "target words" rdm-review-resolved)
                        (2 "drifting" rdm-review-drifted)
                        (3 "Gone text" rdm-review-drifted)
                        (4 "Second line with target words here.\nA drifting phrase."
                           rdm-review-drifted)))
          (let ((ov (rdm-review-test--overlay-of (car want))))
            (should ov)
            (should (equal (rdm-review-test--text ov) (nth 1 want)))
            (should (= (- (overlay-start ov) start)
                       (string-search (nth 1 want) rdm-review-test--old)))
            (should (equal (rdm-review-test--faces ov) (list (nth 2 want)))))))
      (should-not (rdm-review-test--overlay-of 5))
      (should (string-match-p "#5 open  un-placed (resolved)"
                              (rdm-review-test--section))))
    (should (= 1 (length (rdm-review-test--at-calls calls))))))

(ert-deftest rdm-review/other-revision-lists-every-anchor ()
  "A buffer at a revision that is neither current nor the review's
`created_commit' overlays nothing: every anchored comment is listed."
  (rdm-review-test--with (rdm-review-test--placement-reviews)
    (setq rdm-review-test--originals (list (cons "c9" rdm-review-test--old)))
    (with-current-buffer (rdm-open-item rdm-review-test--task "c9")
      (should (equal (rdm-view-body) rdm-review-test--old))
      (should-not (rdm-review-test--overlays))
      (let ((section (rdm-review-test--section)))
        (dolist (id '(1 2 3 4 5))
          (should (string-match-p (format "#%d open  un-placed" id) section)))))
    (should (= 1 (length (rdm-review-test--at-calls calls))))))

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

;;;; Resolution

(ert-deftest rdm-review/resolve-argv-and-head-prefill ()
  "gA offers the comment statuses `describe review' declares and marks the
comment at point with --comment/--status/--reply and an applied commit
pre-filled with the literal HEAD, passed through unchanged for rdm to
resolve in the target's repository, reading the updated review as JSON.
The view is refreshed and the write hook runs."
  (let* ((hooked 0)
         (rdm-after-write-hook (list (lambda () (cl-incf hooked))))
         prefill offered)
    (rdm-review-test--with
        (list (rdm-review-test--review
               "r1" rdm-review-test--task-target
               (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
      (cl-letf (((symbol-function 'completing-read)
                 (lambda (_prompt collection &rest _)
                   (setq offered collection)
                   "addressed"))
                ((symbol-function 'read-string)
                 (lambda (prompt &optional initial &rest _)
                   (if (string-prefix-p "Reply" prompt)
                       "-- fixed"
                     (setq prefill initial)))))
        (with-current-buffer (rdm-open-item rdm-review-test--task)
          (goto-char (overlay-start (rdm-review-test--overlay-of 1)))
          (let* ((before (length calls))
                 (result (call-interactively #'rdm-review-resolve))
                 (update (seq-position (rdm-mock-argvs calls)
                                       '("review" "update")
                                       (lambda (argv words)
                                         (equal (seq-take argv 2) words)))))
            (should (equal offered '("open" "addressed" "wont-fix")))
            (should (equal prefill "HEAD"))
            (should (equal (car (nth update calls))
                           (list "review" "update" "r1" "--comment=1"
                                 "--status=addressed" "--reply=-- fixed"
                                 "--applied-commit=HEAD" "--no-edit"
                                 "--project" "p" "--format" "json")))
            (should (>= update before))
            (should (equal (plist-get result :id) "r1"))
            (let ((comment (aref (plist-get result :comments) 0)))
              (should (equal (plist-get comment :status) "addressed"))
              (should (eq (plist-get comment :closed) t)))
            (setq before update)
            ;; The view re-read the item and its reviews.
            (should (seq-find (lambda (argv) (and (equal (car argv) "review")
                                                  (equal (cadr argv) "list")))
                              (mapcar #'car (nthcdr (1+ before) calls)))))
          (should (= hooked 1)))))))

(ert-deftest rdm-review/resolve-asks-the-declared-flag ()
  "gA asks for an applied commit exactly for the comment status `describe
review' declares it for: moved to `wont-fix' here, `addressed' no
longer asks."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
    (setq rdm-review-test--comment-flags [(:value "wont-fix" :flag "applied-commit")])
    (rdm-context-reset)
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (goto-char (overlay-start (rdm-review-test--overlay-of 1)))
      (dolist (case '(("addressed" nil) ("wont-fix" "HEAD")))
        (let (prefill)
          (cl-letf (((symbol-function 'completing-read)
                     (lambda (&rest _) (car case)))
                    ((symbol-function 'read-string)
                     (lambda (prompt &optional initial &rest _)
                       (if (string-prefix-p "Reply" prompt)
                           ""
                         (setq prefill initial)))))
            (should (equal (nth 3 (rdm-review--read-resolution))
                           (cadr case)))
            (should (equal prefill (cadr case)))))))))

(ert-deftest rdm-review/resolve-wont-fix-without-reply ()
  "wont-fix with an empty reply sends neither --reply nor --applied-commit."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (let ((entry (overlay-get (rdm-review-test--overlay-of 1) 'rdm-review-entry))
            (before (length calls)))
        (should (equal (plist-get (rdm-review-resolve entry "wont-fix" "" nil) :id)
                       "r1"))
        (should (equal (car (nth before calls))
                       '("review" "update" "r1" "--comment=1" "--status=wont-fix"
                         "--no-edit" "--project" "p" "--format" "json")))))))

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
  "Close and dismiss thread --state and return the updated review; a
refused close shows rdm's message."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" rdm-review-test--task-target
             (list (rdm-review-test--comment 1 "café" 17 22 :body "current"))))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (goto-char (overlay-start (rdm-review-test--overlay-of 1)))
      (let ((before (length calls)))
        (should (equal (plist-get (call-interactively #'rdm-review-close) :state)
                       "addressed"))
        (should (equal (car (nth before calls))
                       '("review" "update" "r1" "--state=addressed" "--no-edit"
                         "--project" "p" "--format" "json"))))
      ;; Back to submitted so it is still shown and dismissable.
      (plist-put (car rdm-review-test--reviews) :state "submitted")
      (rdm-review-refresh)
      (goto-char (point-min))
      (search-forward "r1  alice")
      (let ((before (length calls)))
        (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
          (should (equal (plist-get (call-interactively #'rdm-review-dismiss)
                                    :state)
                         "dismissed")))
        (should (equal (car (nth before calls))
                       '("review" "update" "r1" "--state=dismissed" "--no-edit"
                         "--project" "p" "--format" "json"))))
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
                 "r-phase" '(:kind "phase" :roadmap "r1" :stem "phase-2-two"
                                   :ref "phase/r1/phase-2-two"
                                   :item_ref "r1/phase-2-two")
                 (list (rdm-review-test--comment 1 "café" 17 22 :body "current")))
                (rdm-review-test--review "r-plan" '(:kind "plan" :slug "pl" :ref "plan/pl")
                                         nil)
                (rdm-review-test--review "r-change"
                                         '(:kind "change" :head "abc" :ref "change/abc")
                                         nil)))
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

(ert-deftest rdm-review/show-line-range-review ()
  "Show gives a line-range comment its `line N'/`lines N-M' row and
quotes every line of its text with `> '; a resolution without a quote
falls back to the anchor's snippet."
  (let ((unquoted (rdm-review-test--line-comment 2 '(4 . 4) "Fourth only." 40 52)))
    (plist-put (plist-get unquoted :resolution) :quote nil)
    (rdm-review-test--with
        (list (rdm-review-test--review
               "r1" rdm-review-test--task-target
               (list (rdm-review-test--line-comment
                      1 '(2 . 3) "Second line.\nThird line." 5 29)
                     unquoted)))
      (let ((buf (rdm-review-show (car rdm-review-test--reviews))))
        (unwind-protect
            (with-current-buffer buf
              (should (string-match-p
                       (concat "#1  open  resolved\nlines 2-3\n"
                               "> Second line\\.\n> Third line\\.\nComment 1\\.")
                       (buffer-string)))
              (should (string-match-p
                       "#2  open  resolved\nline 4\n> Fourth only\\.\nComment 2\\."
                       (buffer-string))))
          (kill-buffer buf))))))

;;;; Authoring

(defconst rdm-review-test--authoring-body
  "Intro: open the door.\nrepeat me here\nand repeat me there\naaaa end\n"
  "A body with a repeated phrase and \"open\" (which the header's Status
line and the Reviews section also show).")

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

(defun rdm-review-test--body-range (text &optional after)
  "The buffer range (BEG . END) of TEXT in the body.
With AFTER, the TEXT found after the first AFTER."
  (let* ((start (car (rdm-view-body-region)))
         (body (rdm-view-body))
         (offset (string-search text body
                                (if after (string-search after body) 0))))
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

(ert-deftest rdm-review/region-comment-line-ranges ()
  "A region becomes --line: one line, several, partial lines, and a
selection ending just after a newline (which does not take the next
line); no quote is passed.  \"open\" also shows in the
Status line and the Reviews section, but lines count the body alone."
  (rdm-review-test--authoring
    (let ((region (rdm-view-body-region)))
      (should (string-match-p "Status: open" (buffer-substring 1 (car region))))
      (should (string-match-p "open" (buffer-substring (cdr region) (point-max)))))
    (rdm-review-test--comment-on (rdm-review-test--body-range "repeat me" "and")
                                 "Second.")
    (rdm-review-test--comment-on (rdm-review-test--body-range "here\nand") "Span.")
    (rdm-review-test--comment-on (rdm-review-test--body-range "door.\nrepeat")
                                 "Partial.")
    (rdm-review-test--comment-on (rdm-review-test--body-range "repeat me here\n")
                                 "Newline.")
    (rdm-review-test--comment-on (rdm-review-test--body-range "open") "Once.")
    (should (equal (rdm-review-test--calls-of calls "review" "comment")
                   '(("review" "comment" "r1" "--line=3" "--body=Second."
                      "--project" "p" "--no-edit")
                     ("review" "comment" "r1" "--line=2-3" "--body=Span."
                      "--project" "p" "--no-edit")
                     ("review" "comment" "r1" "--line=1-2" "--body=Partial."
                      "--project" "p" "--no-edit")
                     ("review" "comment" "r1" "--line=2" "--body=Newline."
                      "--project" "p" "--no-edit")
                     ("review" "comment" "r1" "--line=1" "--body=Once."
                      "--project" "p" "--no-edit"))))
    ;; No new draft was started: the open one was used.
    (should-not (rdm-review-test--calls-of calls "review" "start"))
    (should (equal rdm-view--at "c0"))))

(ert-deftest rdm-review/region-lines ()
  "Character ranges map to 1-based body lines; the last line is that of
the last selected character."
  (let ((body "one\ntwo\nthree\n"))
    (should (equal (rdm-review-region-lines body 0 3) "1"))
    (should (equal (rdm-review-region-lines body 0 4) "1"))
    (should (equal (rdm-review-region-lines body 0 5) "1-2"))
    (should (equal (rdm-review-region-lines body 5 10) "2-3"))
    (should (equal (rdm-review-region-lines body 8 14) "3"))))

(ert-deftest rdm-review/placed-range-widens-a-point ()
  "An empty range is a point placed on the character there, or at the
body's end the one before it; a non-empty range is kept, a malformed
one or one into an empty body is nil."
  (should (equal (rdm-review--placed-range "one\n\nthree\n" 4 4) '(4 . 5)))
  (should (equal (rdm-review--placed-range "ab\n" 3 3) '(2 . 3)))
  (should (equal (rdm-review--placed-range "one\ntwo\n" 4 7) '(4 . 7)))
  (should-not (rdm-review--placed-range "" 0 0))
  (should-not (rdm-review--placed-range "abc" 2 1))
  (should-not (rdm-review--placed-range "abc" 9 9)))

(ert-deftest rdm-review/blank-line-comment-placed ()
  "Selecting a blank line (just its newline) comments on --line=N; the
comment rdm then reports on it -- an empty snippet at an empty range --
is placed on that line, in the draft's buffer and the current body's,
not listed un-placed."
  (let ((body "one\n\nthree\n"))
    (rdm-review-test--with (list (rdm-review-test--draft))
      (setq rdm-review-test--current body
            rdm-review-test--originals (list (cons "c0" body)))
      (with-current-buffer (rdm-open-item rdm-review-test--task)
        (should (equal rdm-view--at "c0"))
        (let ((start (car (rdm-view-body-region))))
          (rdm-review-test--comment-on (cons (+ start 4) (+ start 5)) "Blank."))
        (should (equal (rdm-review-test--calls-of calls "review" "comment")
                       '(("review" "comment" "r1" "--line=2" "--body=Blank."
                          "--project" "p" "--no-edit"))))
        (plist-put (car rdm-review-test--reviews) :comments
                   (vector (rdm-review-test--line-comment 1 '(2 . 2) "" 4 4)))
        (rdm-review-refresh)
        (let ((ov (rdm-review-test--overlay-of 1)))
          (should ov)
          (should (equal (rdm-review-test--overlay-range ov) '(4 . 5)))
          (should (equal (rdm-review-test--faces ov) '(rdm-review-resolved))))
        (should-not (string-match-p "un-placed" (rdm-review-test--section)))))
    (rdm-review-test--with
        (list (rdm-review-test--review
               "r1" rdm-review-test--task-target
               (list (rdm-review-test--line-comment 1 '(2 . 2) "" 4 4))))
      (setq rdm-review-test--current body)
      (with-current-buffer (rdm-open-item rdm-review-test--task)
        (should-not rdm-view--at)
        (should (equal (rdm-review-test--overlay-range
                        (rdm-review-test--overlay-of 1))
                       '(4 . 5)))
        (should-not (string-match-p "un-placed" (rdm-review-test--section)))))))

(ert-deftest rdm-review/region-refusals-issue-no-call ()
  "A region reaching outside the body and an empty one are refused before
any rdm call."
  (rdm-review-test--authoring
    (let ((before (length calls))
          (body (rdm-view-body-region)))
      (should-error (rdm-review-comment-region (1- (car body)) (+ (car body) 4))
                    :type 'user-error)
      (should-error (rdm-review-comment-region (- (cdr body) 3) (1+ (cdr body)))
                    :type 'user-error)
      (should-error (rdm-review-comment-region (car body) (car body))
                    :type 'user-error)
      (should (= (length calls) before)))))

(ert-deftest rdm-review/document-comment-has-no-quote ()
  "C files a whole-document comment: no --line, no --quote; an empty
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
                     '(("review" "comment" "r-new" "--line=1" "--body=Ok."
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

(defun rdm-review-test--author-argvs (author)
  "The draft-lookup `review list' and `review start' argvs, oldest first.
Opens task t1 and starts a draft with `rdm-review-author' bound to
AUTHOR, under an environment naming other authors (which the package
must not read)."
  (let (argvs)
    (rdm-review-test--with nil
      (let ((rdm-review-author author)
            (process-environment (append '("RDM_REVIEW_AUTHOR=env-other"
                                           "USER=user-other")
                                         process-environment)))
        (with-current-buffer (rdm-open-item rdm-review-test--task)
          (rdm-review-start))
        (setq argvs
              (seq-filter (lambda (argv)
                            (or (equal (seq-take argv 2) '("review" "start"))
                                (and (equal (seq-take argv 2) '("review" "list"))
                                     (member "--state=draft" argv))))
                          (rdm-mock-argvs calls)))))
    argvs))

(ert-deftest rdm-review/author-left-to-rdm-unless-overridden ()
  "With `rdm-review-author' nil the draft lookup passes --mine and no
--author, and `review start' no --author, whatever the environment; set,
both pass it as --author and there is no --mine."
  (let ((argvs (rdm-review-test--author-argvs nil)))
    (should argvs)
    (dolist (argv argvs)
      (should-not (rdm-review-test--flag argv "author")))
    (should (seq-every-p (lambda (argv) (member "--mine" argv))
                         (seq-filter (lambda (a) (equal (cadr a) "list")) argvs)))
    (should (seq-find (lambda (a) (equal (cadr a) "start")) argvs)))
  (let ((argvs (rdm-review-test--author-argvs "pinned")))
    (should argvs)
    (dolist (argv argvs)
      (should (equal (rdm-review-test--flag argv "author") "pinned"))
      (should-not (member "--mine" argv)))))

(ert-deftest rdm-review/first-listed-own-draft-followed ()
  "Of several own drafts the buffer follows the first rdm lists, not one
re-sorted by `created'."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r-first" rdm-review-test--task-target nil
             :state "draft" :verdict nil :author "me"
             :created "2026-09-26T09:00:00Z" :created_commit "c-first")
            (rdm-review-test--review
             "r-later" rdm-review-test--task-target nil
             :state "draft" :verdict nil :author "me"
             :created "2026-09-26T11:00:00Z" :created_commit "c-later"))
    (setq rdm-review-test--originals
          (list (cons "c-first" "First.\n") (cons "c-later" "Later.\n")))
    (with-current-buffer (rdm-open-item rdm-review-test--task)
      (should (equal rdm-view--at "c-first"))
      (should (equal (plist-get rdm-review--draft :id) "r-first"))
      (should (equal (rdm-view-body) "First.\n")))))

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

(defmacro rdm-review-test--dispatching (item &rest body)
  "Run BODY as an `rdm-dispatch' suffix on ITEM would.
Binds `opened' to the number of `rdm-open-item' calls so far."
  (declare (indent 1) (debug t))
  `(let ((opened 0)
         (open (symbol-function 'rdm-open-item)))
     (cl-letf (((symbol-function 'transient-scope) (lambda (&rest _) ,item))
               ((symbol-function 'rdm-open-item)
                (lambda (&rest args) (cl-incf opened) (apply open args)))
               ((symbol-function 'completing-read) (lambda (&rest _) "approve"))
               ((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
       ,@body)))

(defun rdm-review-test--view-buffers ()
  "The live detail buffers."
  (seq-filter (lambda (b) (eq (buffer-local-value 'major-mode b) 'rdm-view-mode))
              (buffer-list)))

(ert-deftest rdm-review/dispatch-start-submit-open-item-buffer ()
  "From a buffer that is not the item's detail buffer, the dispatch start
and submit suffixes open the item's detail buffer, which adopts the draft
\(rendering at its `created_commit'), and submit it with the verdict."
  (rdm-review-test--with nil
    (setq rdm-review-test--originals (list (cons "c-head" "Committed.\n")))
    (with-temp-buffer
      (rdm-review-test--dispatching rdm-review-test--task
        (rdm-review-dispatch-start)
        (should (> opened 0))
        (should-not (derived-mode-p 'rdm-view-mode))
        (let ((views (rdm-review-test--view-buffers)))
          (should (= 1 (length views)))
          (with-current-buffer (car views)
            (should (rdm-review--shows-p rdm-review-test--task))
            (should (equal rdm-view--at "c-head"))
            (should (equal (plist-get rdm-review--draft :id) "r-new"))
            (should (equal (rdm-view-body) "Committed.\n"))))
        (should (equal (rdm-review-test--calls-of calls "review" "start")
                       '(("review" "start" "--on=task/t1" "--author=me" "--no-edit"
                          "--project" "p" "--format" "json"))))
        (rdm-review-dispatch-submit)
        (should (equal (rdm-review-test--calls-of calls "review" "submit")
                       '(("review" "submit" "r-new" "--verdict=approve"
                          "--project" "p" "--no-edit"))))
        (should (= 1 (length (rdm-review-test--view-buffers))))))))

(ert-deftest rdm-review/dispatch-from-item-buffer-opens-nothing ()
  "Run from the item's own detail buffer, the dispatch suffixes author in
that buffer and open no other."
  (rdm-review-test--authoring
    (let ((view (current-buffer)))
      (rdm-review-test--dispatching rdm-review-test--task
        (should (equal (rdm-review-dispatch-start) "r1"))
        (rdm-review-dispatch-submit)
        (should (= opened 0))
        (should (eq (current-buffer) view))
        (should (equal (rdm-review-test--view-buffers) (list view)))
        (should-not (rdm-review-test--calls-of calls "review" "start"))
        (should (equal (rdm-review-test--calls-of calls "review" "submit")
                       '(("review" "submit" "r1" "--verdict=approve"
                          "--project" "p" "--no-edit"))))))))

(ert-deftest rdm-review/dispatch-on-a-roadmap-phase-row ()
  "A review suffix dispatched on a phase row of a roadmap detail buffer
lists that phase's reviews with the `review_ref' the row carries from
`roadmap show', and needs no `phase show' to find it."
  (rdm-review-test--with
      (list (rdm-review-test--review
             "r1" (rdm-review-test--flag-target "phase/r1/phase-1-one") nil))
    (with-current-buffer (rdm-open-item '(:kind roadmap :roadmap "r1"))
      (goto-char (point-min))
      (search-forward "phase-1-one")
      (let ((row (rdm-view-item-at-point)))
        (should (eq (plist-get row :kind) 'phase))
        (should (equal (plist-get row :review-ref) "phase/r1/phase-1-one"))
        (rdm-review-test--dispatching row
          (rdm-review-dispatch-close))
        (should (member '("review" "list" "--on=phase/r1/phase-1-one"
                          "--project" "p" "--format" "json")
                        (rdm-review-test--calls-of calls "review" "list")))
        (should-not (rdm-review-test--calls-of calls "phase" "show"))
        (should (equal (rdm-review-test--calls-of calls "review" "update")
                       '(("review" "update" "r1" "--state=addressed" "--no-edit"
                          "--project" "p" "--format" "json"))))))))

(ert-deftest rdm-review/review-target-is-rdms-review-ref ()
  "The review target is the item's `:review-ref', never a reference built
in Emacs: an item without one is refused and runs no rdm."
  (rdm-review-test--with nil
    (should (equal (rdm-review--on '(:kind task :slug "t1" :review-ref "task/t1"))
                   "task/t1"))
    (should-error (rdm-review--on '(:kind phase :roadmap "r1" :stem "phase-2-two"))
                  :type 'user-error)
    (should-not calls)))

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

(ert-deftest rdm-review/real-review-refs-come-from-rdm ()
  "The real binary's JSON gives a detail buffer and each of its phase rows
the review target `rdm review list --on' accepts."
  (rdm-test-with-real
    (setq process-environment
          (append (list (concat "RDM_ROOT=" root) (concat "RDM_PROJECT=" project))
                  process-environment))
    (setq rdm-bin bin)
    (unwind-protect
        (with-current-buffer (rdm-open-item '(:kind roadmap :roadmap "sample-roadmap"))
          (should (equal (rdm-review--on rdm-view--item) "roadmap/sample-roadmap"))
          (goto-char (point-min))
          (search-forward "phase-2-seed-two")
          (let* ((row (rdm-view-item-at-point))
                 (on (plist-get row :review-ref)))
            (should (equal on "phase/sample-roadmap/phase-2-seed-two"))
            (should (vectorp (rdm-call-sync 'review-list :on on)))))
      (dolist (b (buffer-list))
        (when (eq (buffer-local-value 'major-mode b) 'rdm-view-mode)
          (kill-buffer b))))))

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
          (let* ((comments (plist-get (rdm-call-sync 'review-show id) :comments))
                 (anchor (plist-get (aref comments 0) :anchor)))
            (should (= 1 (length comments)))
            (should (equal (plist-get anchor :anchor_type) "line-range"))
            (should (= (plist-get anchor :start_line) 2))
            (should (= (plist-get anchor :end_line) 2)))
          ;; The overlay layer shows the new comment on the draft's line.
          (let ((ov (rdm-review-test--overlay-of 1)))
            (should ov)
            (should (equal (rdm-review-test--text ov) "Committed line two."))))))))

(ert-deftest rdm-review/real-line-anchor-and-request-changes ()
  "Commenting on a word of the second of two lines sharing it anchors
that whole line: `review show' reports lines 2-2 resolved at that line's
byte range.  Submitting request-changes through the package, with the
verdict picked from those core declares, lands in `review requests' and
the requests picker."
  (rdm-review-test--with-real-author "emacs-real-author-repeat"
    (let* ((slug "emacs-author-repeat-task")
           (item (list :kind 'task :slug slug))
           (first "café alpha one\n")
           (body (concat first "second alpha two\n")))
      (rdm-call-sync 'task-create slug :title "Repeated words" :stdin body)
      (rdm-call-sync 'commit :message "emacs: seed a repeated-words task")
      (with-current-buffer (rdm-open-item item)
        (let ((id (rdm-review-start)))
          (should (equal rdm-view--at (rdm-review-test--plan-head root)))
          (should (equal (rdm-view-body) body))
          (rdm-review-test--comment-on (rdm-review-test--body-range "alpha" "second")
                                       "This one.")
          (let* ((comment (aref (plist-get (rdm-call-sync 'review-show id) :comments) 0))
                 (anchor (plist-get comment :anchor))
                 (res (plist-get comment :resolution)))
            (should (equal (plist-get anchor :anchor_type) "line-range"))
            (should (= (plist-get anchor :start_line) 2))
            (should (= (plist-get anchor :end_line) 2))
            (should (equal (plist-get res :state) "resolved"))
            (should (= (plist-get res :range_start) (string-bytes first)))
            (should (= (plist-get res :range_end)
                       (string-bytes (concat first "second alpha two")))))
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

(defun rdm-review-test--resolution (id)
  "The resolution of the first comment of review ID, read from rdm."
  (plist-get (aref (plist-get (rdm-call-sync 'review-show id) :comments) 0)
             :resolution))

(defun rdm-review-test--commit-body (slug body)
  "Stage BODY as task SLUG's body and commit it."
  (rdm-call-sync 'task-update slug :body body)
  (rdm-call-sync 'commit :message (format "emacs: edit %s" slug)))

(defun rdm-review-test--overlay-range (ov)
  "OV's (START . END) as character offsets into the displayed body."
  (let ((start (car (rdm-view-body-region))))
    (cons (- (overlay-start ov) start) (- (overlay-end ov) start))))

(ert-deftest rdm-review/real-line-range-shifted ()
  "A selection comment is a line-range anchor.  While the draft is open,
its buffer shows the `created_commit' body and keeps the comment on its
original lines from `range_start'/`range_end', drifted once an insertion
above shifts them.  After submitting, the current-body buffer places it
exactly at rdm's `current' while only lines below change, and lists it
un-placed once an insertion above makes `current' null."
  (rdm-review-test--with-real-author "emacs-real-author-lines"
    (let* ((slug "emacs-author-lines-task")
           (item (list :kind 'task :slug slug))
           (body "Intro — naïve.\nSecond 日本語 line.\nThird line.\nFourth line.\n")
           (lines "Second 日本語 line.\nThird line."))
      (rdm-call-sync 'task-create slug :title "Line ranges" :stdin body)
      (rdm-call-sync 'commit :message "emacs: seed a line-range task")
      (with-current-buffer (rdm-open-item item)
        (let ((id (rdm-review-start)))
          (rdm-review-test--comment-on (rdm-review-test--body-range "日本語 line.\nThird")
                                       "These two.")
          (let ((anchor (plist-get (aref (plist-get (rdm-call-sync 'review-show id)
                                                    :comments)
                                         0)
                                   :anchor)))
            (should (equal (plist-get anchor :anchor_type) "line-range"))
            (should (= (plist-get anchor :start_line) 2))
            (should (= (plist-get anchor :end_line) 3)))
          (let ((ov (rdm-review-test--overlay-of 1)))
            (should (equal (rdm-review-test--text ov) lines))
            (should (equal (rdm-review-test--faces ov) '(rdm-review-resolved))))
          ;; The draft buffer, after an insertion above the range is committed.
          (rdm-review-test--commit-body slug (concat "New top.\n" body))
          (revert-buffer)
          (should (equal rdm-view--at (plist-get rdm-review--draft :commit)))
          (should (equal (rdm-view-body) body))
          (let ((res (rdm-review-test--resolution id))
                (ov (rdm-review-test--overlay-of 1)))
            (should (equal (plist-get res :state) "drifted"))
            (should (equal (plist-get res :body) "original"))
            (should-not (plist-get res :current))
            (should ov)
            (should (equal (rdm-review-test--overlay-range ov)
                           (rdm-review--char-range body
                                                   (plist-get res :range_start)
                                                   (plist-get res :range_end))))
            (should (equal (rdm-review-test--text ov) lines))
            (should (equal (rdm-review-test--faces ov) '(rdm-review-drifted))))
          ;; Restore the body, then submit: the buffer shows the current body.
          (rdm-review-test--commit-body slug body)
          (cl-letf (((symbol-function 'completing-read) (lambda (&rest _) "comment"))
                    ((symbol-function 'y-or-n-p) (lambda (&rest _) t)))
            (call-interactively #'rdm-review-submit))
          (should-not rdm-view--at)
          ;; An edit below the range: placed at rdm's `current'.
          (let ((below (concat "Intro — naïve.\nSecond 日本語 line.\nThird line.\n"
                               "Fourth line, changed.\nFifth.\n")))
            (rdm-review-test--commit-body slug below)
            (revert-buffer)
            (should (equal (rdm-view-body) below))
            (let* ((current (plist-get (rdm-review-test--resolution id) :current))
                   (ov (rdm-review-test--overlay-of 1)))
              (should current)
              (should ov)
              (should (equal (rdm-review-test--overlay-range ov)
                             (rdm-review--char-range below
                                                     (plist-get current :range_start)
                                                     (plist-get current :range_end))))
              (should (equal (rdm-review-test--text ov) lines))))
          ;; An insertion above the range: `current' null, listed un-placed.
          (rdm-review-test--commit-body slug (concat "New top.\n" body))
          (revert-buffer)
          (let ((res (rdm-review-test--resolution id)))
            (should (equal (plist-get res :state) "drifted"))
            (should-not (plist-get res :current)))
          (should-not (rdm-review-test--overlays))
          (should (string-match-p "#1 open  un-placed (drifted) lines 2-3"
                                  (rdm-review-test--section))))))))

(ert-deftest rdm-review/real-blank-line-comment-placed ()
  "A comment on a blank line, which rdm resolves to an empty range, is
placed on that line in the draft's buffer, not listed un-placed."
  (rdm-review-test--with-real-author "emacs-real-author-blank"
    (let* ((slug "emacs-author-blank-task")
           (item (list :kind 'task :slug slug))
           (body "one\n\nthree\n"))
      (rdm-call-sync 'task-create slug :title "Blank line" :stdin body)
      (rdm-call-sync 'commit :message "emacs: seed a blank-line task")
      (with-current-buffer (rdm-open-item item)
        (let ((id (rdm-review-start)))
          (should (equal (rdm-view-body) body))
          (let ((start (car (rdm-view-body-region))))
            (rdm-review-test--comment-on (cons (+ start 4) (+ start 5)) "Blank."))
          (let ((res (rdm-review-test--resolution id)))
            (should (equal (plist-get res :state) "resolved"))
            (should (= (plist-get res :range_start) (plist-get res :range_end))))
          (should (equal (rdm-review-test--overlay-range
                          (rdm-review-test--overlay-of 1))
                         '(4 . 5)))
          (should-not (string-match-p "un-placed" (rdm-review-test--section))))))))

(ert-deftest rdm-review/real-drifted-quote ()
  "A quote comment rdm reports drifted is placed only from the review
JSON: the current-body buffer at rdm's `current' (or listed un-placed
when it is null), a buffer at the review's `created_commit' at the
original range."
  (rdm-review-test--with-real-author "emacs-real-drifted-quote"
    (let* ((slug "emacs-drifted-quote-task")
           (item (list :kind 'task :slug slug))
           (body "one alpha\ntwo alpha\nthree\n")
           (changed "one alpha\nzwo alpha\nthree\n")
           id commit)
      (rdm-call-sync 'task-create slug :title "Drifted quote" :stdin body)
      (rdm-call-sync 'commit :message "emacs: seed a drifted-quote task")
      (let ((out (rdm-review-test--rdm bin "review" "start" "--on"
                                       (concat "task/" slug) "--author" "cli-author"
                                       "--no-edit" "--project" project)))
        (should (string-match "Started review '\\([^']+\\)'" out))
        (setq id (match-string 1 out)))
      (rdm-review-test--rdm bin "review" "comment" id "--quote" "two alpha"
                            "--body" "Two." "--no-edit" "--project" project)
      (rdm-review-test--rdm bin "review" "submit" id "--verdict" "comment"
                            "--no-edit" "--project" project)
      (rdm-review-test--commit-body slug changed)
      (setq commit (plist-get (rdm-call-sync 'review-show id) :created_commit))
      (let ((res (rdm-review-test--resolution id)))
        (should (equal (plist-get res :state) "drifted"))
        (with-current-buffer (rdm-open-item item)
          (should-not rdm-view--at)
          (should (equal (rdm-view-body) changed))
          (let ((current (plist-get res :current))
                (ov (rdm-review-test--overlay-of 1)))
            (if current
                (progn
                  (should ov)
                  (should (equal (rdm-review-test--overlay-range ov)
                                 (rdm-review--char-range
                                  changed (plist-get current :range_start)
                                  (plist-get current :range_end))))
                  (should (equal (rdm-review-test--faces ov) '(rdm-review-drifted))))
              (should-not ov)
              (should (string-match-p "#1 open  un-placed (drifted)"
                                      (rdm-review-test--section))))))
        (with-current-buffer (rdm-open-item item commit)
          (should (equal (rdm-view-body) body))
          (let ((ov (rdm-review-test--overlay-of 1)))
            (should ov)
            (should (equal (rdm-review-test--overlay-range ov)
                           (rdm-review--char-range body
                                                   (plist-get res :range_start)
                                                   (plist-get res :range_end))))
            (should (equal (rdm-review-test--text ov) "two alpha"))
            (should (equal (rdm-review-test--faces ov) '(rdm-review-drifted)))))))))

(provide 'rdm-review-test)
;;; rdm-review-test.el ends here
