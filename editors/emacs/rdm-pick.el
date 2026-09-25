;;; rdm-pick.el --- Pickers, embark actions and rdm-next  -*- lexical-binding: t; -*-

;; This file is not part of GNU Emacs.

;;; Commentary:

;; `completing-read' pickers over the resolved project's plan data:
;;
;; - `rdm-roadmap' picks a roadmap and opens it; with a prefix argument it
;;   drills down into the roadmap's phases.
;; - `rdm-phase' picks a phase of a roadmap.
;; - `rdm-task' picks an open or in-progress task (all tasks with a prefix
;;   argument).
;; - `rdm-search' is a live search: with consult, every minibuffer input
;;   runs `rdm search' (rdm's search is already fuzzy, so there is no
;;   client-side matching); without it, it reads a query and offers the
;;   results through plain `completing-read'.
;; - `rdm-next' opens the next actionable phase of a roadmap.
;;
;; Candidates carry an `rdm-item' text property (see rdm-view.el) and the
;; completion `category' `rdm-item', whose `annotation-function' shows
;; status and tags (kind and tags for search hits) with or without
;; marginalia.  Embark finds its actions for that category in
;; `rdm-item-embark-map'.  Neither consult nor embark is required.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'subr-x)
(require 'rdm-core)
(require 'rdm-view)

(defvar embark-keymap-alist)
(declare-function consult--read "ext:consult" (table &rest options))
(declare-function consult--dynamic-collection "ext:consult" (fun &rest keys))

(defcustom rdm-search-use-consult t
  "Whether `rdm-search' uses consult's live search when consult is available."
  :type 'boolean
  :group 'rdm)

;;;; Candidates

(defun rdm-pick--annotation (label tags)
  "The annotation text for LABEL (a status or kind) and TAGS."
  (let ((tags (rdm-view--tags-string tags)))
    (concat (or label "") (if tags (format "  [%s]" tags) ""))))

(defun rdm-pick--candidate (text item label)
  "Register candidate TEXT for ITEM, annotated with LABEL and ITEM's tags."
  (rdm-item-register
   text (append item (list :annotation
                           (rdm-pick--annotation label (plist-get item :tags))))))

(defun rdm-pick-roadmap-candidates ()
  "Candidates for the project's roadmaps, annotated with progress and tags."
  (seq-map (lambda (r)
             (rdm-pick--candidate
              (plist-get r :slug)
              (list :kind 'roadmap :roadmap (plist-get r :slug)
                    :title (plist-get r :title) :tags (plist-get r :tags))
              (plist-get r :progress)))
           (rdm-call-sync 'roadmap-list)))

(defun rdm-pick-phase-candidates (roadmap)
  "Candidates for ROADMAP's phases, annotated with their status."
  (seq-map (lambda (p)
             (rdm-pick--candidate
              (plist-get p :stem)
              (list :kind 'phase :roadmap roadmap :stem (plist-get p :stem)
                    :title (plist-get p :title) :status (plist-get p :status))
              (plist-get p :status)))
           (rdm-call-sync 'phase-list :roadmap roadmap)))

(defun rdm-pick-task-candidates (&optional all)
  "Candidates for open and in-progress tasks, or every task when ALL."
  (seq-map (lambda (task)
             (rdm-pick--candidate
              (plist-get task :slug)
              (list :kind 'task :slug (plist-get task :slug)
                    :title (plist-get task :title)
                    :status (plist-get task :status)
                    :tags (plist-get task :tags))
              (plist-get task :status)))
           (rdm-call-sync 'task-list :status (and all "all"))))

(defun rdm-pick--search-item (hit)
  "The item for search HIT, or a bare item of its kind."
  (let ((kind (intern (plist-get hit :kind)))
        (id (plist-get hit :identifier)))
    (append
     (pcase kind
       ('roadmap (list :kind kind :roadmap id))
       ('phase (let ((slash (string-search "/" id)))
                 (list :kind kind :roadmap (substring id 0 slash)
                       :stem (substring id (1+ slash)))))
       ('task (list :kind kind :slug id))
       (_ (list :kind kind :identifier id)))
     (list :title (plist-get hit :title) :tags (plist-get hit :tags)))))

(defconst rdm-pick-search-kinds '("roadmap" "phase" "task")
  "The search hit kinds a picker offers: the ones a detail buffer can open.
`rdm search' also returns other kinds (reviews); those are dropped.")

(defun rdm-pick-search-candidates (query)
  "Candidates for `rdm search QUERY', annotated with kind and tags.
Only hits of `rdm-pick-search-kinds' are offered."
  (if (string-empty-p (string-trim query))
      nil
    (let (seen)
      (seq-map (lambda (hit)
                 (let* ((id (plist-get hit :identifier))
                        (kind (plist-get hit :kind))
                        (text (if (member id seen) (format "%s (%s)" id kind) id)))
                   (push text seen)
                   (rdm-pick--candidate text (rdm-pick--search-item hit) kind)))
               (seq-filter (lambda (hit)
                             (member (plist-get hit :kind) rdm-pick-search-kinds))
                           (rdm-call-sync 'search query))))))

;;;; Reading

(defun rdm-pick-annotate (candidate)
  "The annotation for CANDIDATE: its title, status or kind, and tags."
  (when-let* ((item (rdm-item-from candidate)))
    (concat "  "
            (propertize (or (plist-get item :title) "") 'face 'completions-annotations)
            "  " (or (plist-get item :annotation) ""))))

(defun rdm-pick-table (candidates)
  "A completion table over CANDIDATES in category `rdm-item'."
  (lambda (string pred action)
    (if (eq action 'metadata)
        '(metadata (category . rdm-item)
                   (annotation-function . rdm-pick-annotate))
      (complete-with-action action candidates string pred))))

(defun rdm-pick-read (prompt candidates)
  "Read one of CANDIDATES with PROMPT; return its item."
  (unless candidates (user-error "rdm: nothing to pick"))
  (rdm-item--require
   (completing-read prompt (rdm-pick-table candidates) nil t)))

(defun rdm-pick-read-roadmap (&optional prompt)
  "Read a roadmap with PROMPT; return its slug."
  (plist-get (rdm-pick-read (or prompt "Roadmap: ")
                            (rdm-pick-roadmap-candidates))
             :roadmap))

;;;; Commands

;;;###autoload
(defun rdm-roadmap (&optional drill-down)
  "Pick a roadmap and open it; with DRILL-DOWN (prefix), pick one of its phases."
  (interactive "P")
  (let ((roadmap (rdm-pick-read-roadmap)))
    (if drill-down
        (rdm-phase roadmap)
      (rdm-open-item (list :kind 'roadmap :roadmap roadmap)))))

;;;###autoload
(defun rdm-phase (roadmap)
  "Pick a phase of ROADMAP and open it.  Prompts for the roadmap."
  (interactive (list (rdm-pick-read-roadmap)))
  (rdm-open-item (rdm-pick-read (format "Phase of %s: " roadmap)
                                (rdm-pick-phase-candidates roadmap))))

;;;###autoload
(defun rdm-task (&optional all)
  "Pick an open or in-progress task and open it; ALL (prefix) lists every task."
  (interactive "P")
  (rdm-open-item (rdm-pick-read "Task: " (rdm-pick-task-candidates all))))

(defun rdm-pick-search-collection ()
  "A consult dynamic collection running `rdm search' for each input.
The search runs in the current buffer's rdm context, not the
minibuffer's; a failing search yields no candidates."
  (let ((buf (current-buffer)))
    (consult--dynamic-collection
     (lambda (input)
       (with-current-buffer (if (buffer-live-p buf) buf (current-buffer))
         (condition-case nil
             (rdm-pick-search-candidates input)
           (error nil)))))))

(defun rdm-pick--search-read ()
  "Read a search hit, live through consult when available; return its item."
  (if (and rdm-search-use-consult (require 'consult nil t))
      (rdm-item--require
       (consult--read (rdm-pick-search-collection)
                      :prompt "rdm search: "
                      :category 'rdm-item
                      :annotate #'rdm-pick-annotate
                      :require-match t
                      :sort nil))
    (rdm-pick-read "Result: "
                   (rdm-pick-search-candidates (read-string "rdm search: ")))))

;;;###autoload
(defun rdm-search ()
  "Search the project's plan data and open the chosen hit."
  (interactive)
  (rdm-open-item (rdm-pick--search-read)))

(defun rdm-pick--worktree-roadmap ()
  "The roadmap of the rdm worktree holding `default-directory', or nil."
  (let ((item (plist-get (ignore-errors (rdm-worktree-item)) :item)))
    (when (and (stringp item) (not (string-prefix-p "task/" item)))
      (car (split-string item "/")))))

;;;###autoload
(defun rdm-next (roadmap)
  "Open the next actionable phase of ROADMAP.
Interactively, ROADMAP is the current worktree's roadmap, else picked;
a prefix argument always picks."
  (interactive
   (list (or (and (not current-prefix-arg) (rdm-pick--worktree-roadmap))
             (rdm-pick-read-roadmap "Next phase of roadmap: "))))
  (let ((next (rdm-call-sync 'next :roadmap roadmap)))
    (pcase (plist-get next :result)
      ("phase"
       (rdm-open-item (list :kind 'phase :roadmap (plist-get next :roadmap)
                            :stem (plist-get next :stem))))
      ("blocked-on-dependencies"
       (user-error "rdm: %s is blocked on %s" roadmap
                   (mapconcat (lambda (u) (if (stringp u) u (format "%S" u)))
                              (plist-get next :unmet) ", ")))
      (_ (message "rdm: nothing actionable in %s" roadmap) nil))))

;;;; Embark

(defun rdm-pick--read-item (prompt)
  "Read an item candidate with PROMPT: how embark hands actions a target."
  (rdm-item--require (read-string prompt)))

(defun rdm-item-open (item)
  "Open ITEM (an embark target)."
  (interactive (list (rdm-pick--read-item "Open rdm item: ")))
  (rdm-open-item item))

(defun rdm-item-open-other-window (item)
  "Open ITEM (an embark target) in another window."
  (interactive (list (rdm-pick--read-item "Open rdm item: ")))
  (rdm-open-item item nil t))

(defun rdm-item-copy-identifier (item)
  "Copy ITEM's identifier (an embark target)."
  (interactive (list (rdm-pick--read-item "Copy identifier of: ")))
  (rdm-copy-identifier item))

(defun rdm-item-copy-done-line (item)
  "Copy ITEM's `Done:' commit line (an embark target)."
  (interactive (list (rdm-pick--read-item "Copy Done: line of: ")))
  (rdm-copy-done-line item))

(defvar-keymap rdm-item-embark-map
  :doc "Embark actions for `rdm-item' candidates.
Later phases (and users) add actions with `keymap-set'; each action
command reads its target with `read-string' and recovers the item
through `rdm-item-from'."
  "RET" #'rdm-item-open
  "o" #'rdm-item-open-other-window
  "w" #'rdm-item-copy-identifier
  "d" #'rdm-item-copy-done-line)

(with-eval-after-load 'embark
  (add-to-list 'embark-keymap-alist '(rdm-item rdm-item-embark-map)))

(provide 'rdm-pick)
;;; rdm-pick.el ends here
